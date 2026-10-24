//! Checking one file (spec chapter 2): scopes and locals, statements,
//! function bodies with their returns, and class bodies.

use super::env::{DefKind, Env, FnSig, Scope};
use super::narrow::Flow;
use super::query::Mode;
use super::types::*;
use super::types::{Field, Func, Types};
use rustc_hash::{FxHashMap, FxHashSet};
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::view::Func as FuncNode;
use wowlua_syntax::view::*;

/// The prefix of a file-level local's type among a module's exports
/// (`import internal`, spec Tooling §16.1.3.1).
pub const INTERNAL: &str = "\u{1}internal:";

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
    /// Where it is declared (the name's start), if in the source.
    pub decl: Option<u32>,
    /// What closures see of an unannotated local (spec §2.4.1): its
    /// narrowed types where functions were made, and the values assigned
    /// to it after that; `never` before.
    pub assigned: TypeId,
    /// A function was made where it is in scope.
    pub captured: bool,
    /// Declared with `nil` (or no value): that `nil` is not counted for a
    /// function made before a value is assigned (a forward declaration).
    pub nil_init: bool,
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
    /// Globals other modules' `@global` exports create, with the exports'
    /// types.
    pub global_types: FxHashMap<Box<str>, TypeId>,
    /// Imported bindings: local name → type.
    pub imports: FxHashMap<Box<str>, TypeId>,
    /// The counts of imported functions (W2062), by local name.
    pub import_caps: FxHashMap<Box<str>, usize>,
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

/// The locals in scope: every name's variables, innermost last, and the
/// names declared since each scope began (the file's top level is not
/// popped).
#[derive(Default)]
struct Scopes {
    /// Name → symbol.
    ids: FxHashMap<Box<str>, u32>,
    /// By symbol: its variables in scope, innermost (latest) last.
    stacks: Vec<Vec<VarId>>,
    /// The symbols declared, in order.
    log: Vec<u32>,
    /// `log.len()` where each open scope began.
    marks: Vec<usize>,
}

impl Scopes {
    fn declare(&mut self, name: &str, id: VarId) {
        let sym = match self.ids.get(name) {
            Some(&sym) => sym,
            None => {
                let sym = self.stacks.len() as u32;
                self.ids.insert(name.into(), sym);
                self.stacks.push(Vec::new());
                sym
            }
        };
        self.stacks[sym as usize].push(id);
        self.log.push(sym);
    }

    fn lookup(&self, name: &str) -> Option<VarId> {
        let &sym = self.ids.get(name)?;
        self.stacks[sym as usize].last().copied()
    }
}

pub struct Checker<'e, 'p> {
    pub env: &'e mut Env<'p>,
    pub mode: Mode,
    pub scope: Scope,
    pub(crate) vars: Vec<Var>,
    scopes: Scopes,
    pub(crate) flow: Flow,
    /// Fields of locals that conditions narrowed (`a.b`, `a.b.c`, spec
    /// §2.5): each path is a synthetic variable, so flows narrow and join
    /// it like a local. By the root local, then the dotted path.
    pub(crate) field_vars: FxHashMap<VarId, FxHashMap<Box<str>, VarId>>,
    /// The current path can still run.
    pub(crate) live: bool,
    fns: Vec<FnCtx>,
    pub depth: u32,
    /// While a parameter's default is checked: that parameter and the ones
    /// after it, not set yet (reading one is E1004).
    pub(crate) unset_params: FxHashSet<VarId>,
    pub diagnostics: Vec<Diagnostic>,
    pub extra_globals: &'e FxHashSet<String>,
    pub assigned_globals: &'e FxHashSet<String>,
    /// See [`FileInfo::global_types`].
    pub global_types: FxHashMap<Box<str>, TypeId>,
    /// Types of expressions already checked, by span.
    pub(crate) expr_types: FxHashMap<(u32, u32), TypeId>,
    /// Links of optional chains: their type before the short-circuit's
    /// `nil`, which the rest of the chain continues from.
    pub(crate) chain_inner: FxHashMap<(u32, u32), TypeId>,
    /// Classes whose bodies are being checked: (def, static member).
    pub(crate) classes: Vec<(DefId, bool)>,
    /// The class whose constructor is checked: its readonly fields may be
    /// assigned there, nowhere else (spec §3.1.1).
    pub(crate) ctor_of: Option<DefId>,
    /// Exported values of this module.
    pub exports: FxHashMap<Box<str>, TypeId>,
    /// The exported locals, in order: their types are recorded once the
    /// top level has run (fields added after the declaration included).
    export_vars: Vec<(Box<str>, VarId)>,
    /// By the type `setmetatable` gave: the fields read through the
    /// metatable's `__index` (spec §2.4.6). A table's own definition
    /// replaces them, as it does at run time.
    pub(crate) fallback_fields: FxHashMap<TypeId, FxHashSet<Box<str>>>,
    /// Name spans of accesses to renamable methods (`rename-private`), with
    /// the class that declares the method.
    pub private_methods: FxHashMap<wowlua_span::Span, DefId>,
    /// Classes whose instances or class tables this file stores in globals.
    pub global_classes: FxHashSet<DefId>,
    reassigned: FxHashSet<u32>,
    pub(crate) assigned_in_closures: FxHashSet<u32>,
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
    /// The most values a `local function` can give, counted from its own
    /// `return`s when each gives a fixed number (W2062, decision S40).
    pub(crate) result_caps: FxHashMap<VarId, usize>,
    /// The same for the methods of a file-level table (`function M:f()`,
    /// `function M.f()`): by the table's variable and the method's name.
    pub(crate) method_caps: FxHashMap<(VarId, Box<str>), usize>,
    /// A method's `self`: the table it was defined on (`function M:f()`).
    pub(crate) self_owners: FxHashMap<VarId, VarId>,
    /// The table of the method whose body is checked next.
    pending_self_owner: Option<VarId>,
    /// `overload function` signatures waiting for their implementation:
    /// (name, signature, where).
    pending_overloads: Vec<(Box<str>, TypeId, Span)>,
    /// Uses of a call whose count wasn't known yet (a method defined
    /// further down): judged at the file's end.
    pub(crate) pending_caps: Vec<(super::expr::CapKey, String, super::expr::CapUse)>,
    /// The counts of this file's exported functions (by export name, the
    /// file-level locals under `INTERNAL` + name), for importers.
    pub export_caps: FxHashMap<Box<str>, usize>,
    /// The count of the function body just checked (see `result_caps`).
    last_result_cap: Option<usize>,
    /// The loops being checked, innermost last: the flows of their
    /// `break`s and `continue`s.
    loops: Vec<LoopFlows>,
    /// While a loop is checked on trial: the base and assigned types its
    /// assignments replaced, to restore if the trial fails.
    base_log: Option<Vec<(VarId, TypeId, TypeId)>>,
    /// The types closures see of the unannotated locals a closure
    /// assigns (or captured and assigned later), by declaration: the
    /// previous pass's unions of their assigned values (spec §2.4.1).
    pub(crate) closure_seeds: FxHashMap<u32, TypeId>,
    /// A closure read such a local in this pass.
    pub(crate) closure_hit: std::cell::Cell<bool>,
}

/// The flows that leave a loop body early.
#[derive(Default)]
struct LoopFlows {
    breaks: Vec<Flow>,
    continues: Vec<Flow>,
}

/// What a loop's trial check restores when it fails.
struct LoopTrial {
    flow: Flow,
    field_vars: FxHashMap<VarId, FxHashMap<Box<str>, VarId>>,
    live: bool,
    in_dead: bool,
    guard: super::flavors::Guard,
    vars: usize,
    diagnostics: usize,
    env_diagnostics: usize,
    returns: usize,
    base_log: Option<Vec<(VarId, TypeId, TypeId)>>,
}

/// What checking a file reads of its syntax before and after walking it:
/// the same in every flavor, so a build prepares it once per file (on
/// worker threads, `FileInput::prepared`).
#[derive(Debug, Default)]
pub struct Prepared {
    reassigned: FxHashSet<u32>,
    assigned_in_closures: FxHashSet<u32>,
    assigned_later: FxHashSet<u32>,
    dynamic_env: bool,
    /// The `typeName(x)` arguments, in walk order (the callee's name
    /// decides; lowering only uses entries of `wowlua:debug` intrinsics).
    pub(crate) type_name_args: Vec<wowlua_span::Span>,
    /// The globals `CreateFrame(type, "Name", …)` creates, in walk order.
    pub(crate) created_frames: Vec<String>,
}

/// [`Prepared`] for a file.
pub fn prepare(parse: &wowlua_syntax::Parse) -> Prepared {
    let (reassigned, assigned_in_closures, assigned_later) =
        reassigned_in_closures(parse.view().block());
    let mut type_name_args = Vec::new();
    let mut created_frames = Vec::new();
    wowlua_syntax::visit::walk_block(parse.view().block(), &mut |e| {
        let ExprKind::Call { callee, args } = e.kind() else {
            return;
        };
        if (matches!(callee.kind(), ExprKind::Name(n) if n.text() == "typeName")
            | matches!(callee.kind(), ExprKind::Field { name, .. } if name.text() == "typeName"))
            && let Some(arg) = args.exprs().first()
        {
            type_name_args.push(arg.span());
        }
        if matches!(callee.kind(), ExprKind::Name(n) if n.text() == "CreateFrame")
            && let Some(ExprKind::String(lit)) = args.exprs().get(1).map(|a| a.kind())
        {
            created_frames.push(String::from_utf8_lossy(&lit.value()).into_owned());
        }
    });
    Prepared {
        reassigned,
        assigned_in_closures,
        assigned_later,
        dynamic_env: calls_setfenv(parse.view().block()),
        type_name_args,
        created_frames,
    }
}

impl<'e, 'p> Checker<'e, 'p> {
    pub fn new(env: &'e mut Env<'p>, info: &FileInfo<'e>, prepared: &Prepared) -> Self {
        let Prepared {
            reassigned,
            assigned_in_closures,
            assigned_later,
            dynamic_env,
            type_name_args: _,
            created_frames: _,
        } = prepared;
        let (reassigned, assigned_in_closures, assigned_later, dynamic_env) = (
            reassigned.clone(),
            assigned_in_closures.clone(),
            assigned_later.clone(),
            *dynamic_env,
        );
        let mut c = Checker {
            env,
            mode: info.mode,
            scope: Scope {
                module: Some(info.index),
                ..Scope::default()
            },
            vars: Vec::new(),
            scopes: Scopes::default(),
            flow: Flow::default(),
            field_vars: FxHashMap::default(),
            live: true,
            fns: Vec::new(),
            depth: 0,
            unset_params: FxHashSet::default(),
            diagnostics: Vec::new(),
            extra_globals: info.extra_globals,
            assigned_globals: info.assigned_globals,
            global_types: info.global_types.clone(),
            expr_types: FxHashMap::default(),
            chain_inner: FxHashMap::default(),
            classes: Vec::new(),
            ctor_of: None,
            exports: FxHashMap::default(),
            export_vars: Vec::new(),
            fallback_fields: FxHashMap::default(),
            private_methods: FxHashMap::default(),
            global_classes: FxHashSet::default(),
            reassigned,
            assigned_in_closures,
            assigned_later,
            pure_lua: info.pure_lua,
            top_vararg: None,
            dynamic_env,
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
            result_caps: FxHashMap::default(),
            method_caps: FxHashMap::default(),
            self_owners: FxHashMap::default(),
            pending_self_owner: None,
            pending_overloads: Vec::new(),
            pending_caps: Vec::new(),
            export_caps: FxHashMap::default(),
            last_result_cap: None,
            loops: Vec::new(),
            base_log: None,
            closure_seeds: FxHashMap::default(),
            closure_hit: std::cell::Cell::new(false),
        };
        for (name, &t) in &info.imports {
            let id = c.declare(name, Some(t), t, true, None);
            c.import_vars.insert(name.clone(), id);
            if let Some(&cap) = info.import_caps.get(name) {
                c.result_caps.insert(id, cap);
            }
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
        let mut msg = format!(
            "`{}` is not assignable to `{}`",
            self.env.display(s),
            self.env.display(t)
        );
        if let Some(why) = self.record_mismatch(s, t) {
            msg.push_str(": ");
            msg.push_str(&why);
        }
        self.report("E2001", span, msg);
    }

    /// Why record `s` does not fit the record of `t` (`t` may be optional):
    /// the first required field it lacks or whose type does not fit.
    fn record_mismatch(&mut self, s: TypeId, t: TypeId) -> Option<String> {
        let Ty::Record(source) = self.env.types.get(s).clone() else {
            return None;
        };
        let target = self.env.types.non_nil(t);
        let Ty::Record(target) = self.env.types.get(target).clone() else {
            return None;
        };
        for (name, f) in target.fields.iter() {
            match source.fields.get(name) {
                None if !f.optional => {
                    let mut why = format!("the field `{name}` is missing");
                    // `x: T?` must be present; `x?: T` may be absent (§2.2.4).
                    if self.env.types.is_optional(f.ty) {
                        let inner = self.env.types.non_nil(f.ty);
                        why.push_str(&format!(
                            " (`{name}: {0}?` must be present; write `{name}?: {0}` for a field that may be absent)",
                            self.env.display(inner)
                        ));
                    }
                    return Some(why);
                }
                Some(sf) if !self.env.assignable(sf.ty, f.ty, self.mode) => {
                    return Some(format!(
                        "the field `{name}` is `{}`, not `{}`",
                        self.env.display(sf.ty),
                        self.env.display(f.ty)
                    ));
                }
                _ => {}
            }
        }
        None
    }

    pub fn check_assignable(&mut self, span: Span, s: TypeId, t: TypeId) {
        if !self.env.assignable(s, t, self.mode) {
            self.mismatch(span, s, t);
        }
    }

    // ------------------------------------------------------------ scopes --

    pub fn push_scope(&mut self) {
        self.scopes.marks.push(self.scopes.log.len());
    }

    pub fn pop_scope(&mut self) {
        let Some(mark) = self.scopes.marks.pop() else {
            return;
        };
        // The flow forgets the last variable of each name declared in the
        // scope (the one its name found).
        let mut forgotten: FxHashSet<u32> = FxHashSet::default();
        while self.scopes.log.len() > mark {
            let sym = self.scopes.log.pop().expect("logged");
            let v = self.scopes.stacks[sym as usize].pop().expect("declared");
            if forgotten.insert(sym) {
                self.flow.remove(&v);
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
            None if widen => self.env.types.widen_value(ty),
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
            decl: span.map(|s| s.start),
            assigned: Types::NEVER,
            captured: false,
            nil_init: declared.is_none() && base == Types::NIL,
        });
        self.scopes.declare(name, id);
        if declared.is_none() {
            // Unannotated locals hold widened literals (spec §2.4).
            let t = if is_const || !widen {
                ty
            } else {
                self.env.types.widen_value(ty)
            };
            self.flow.insert(id, t);
        }
        id
    }

    pub fn lookup_var(&self, name: &str) -> Option<VarId> {
        self.scopes.lookup(name)
    }

    /// An annotation's type; `typeof(expr)` parts are inferred first
    /// (spec §2.2.7).
    pub fn annotation(&mut self, t: Type<'_>) -> TypeId {
        let mut exprs: Vec<Expr<'_>> = Vec::new();
        typeof_exprs(t, &mut exprs);
        for e in exprs {
            let ty = self.infer(e);
            let ty = self.env.types.widen(ty);
            self.env
                .typeof_types
                .insert((e.span().start, e.span().end), ty);
        }
        let scope = self.scope.clone();
        self.env.resolve(t, &scope)
    }

    /// A function's signature where its body is checked: the `typeof(expr)`
    /// parts of its annotations are inferred first, as in [`Self::annotation`]
    /// (decision L62).
    pub fn func_sig<'t>(&mut self, f: &FnSig<'t>) -> Func {
        fn vararg_type(v: VarargType<'_>) -> Option<Type<'_>> {
            match v {
                VarargType::Type(t) | VarargType::Expand(t) => Some(t),
                VarargType::Pack(_) => None,
            }
        }
        let mut types: Vec<Type<'t>> = f.params.iter().filter_map(|p| p.ty).collect();
        if let Some(t) = f.vararg.and_then(|v| v.ty).and_then(vararg_type) {
            types.push(t);
        }
        match f.ret {
            Some(RetType::Single(t)) | Some(RetType::Predicate { ty: t, .. }) => types.push(t),
            Some(RetType::List {
                types: list,
                vararg,
                ..
            }) => {
                types.extend(list.iter());
                types.extend(vararg.and_then(vararg_type));
            }
            Some(RetType::Pack(_)) | None => {}
        }
        let mut exprs: Vec<Expr<'t>> = Vec::new();
        for t in types {
            typeof_exprs(t, &mut exprs);
        }
        for e in exprs {
            let ty = self.infer(e);
            let ty = self.env.types.widen(ty);
            self.env
                .typeof_types
                .insert((e.span().start, e.span().end), ty);
        }
        let scope = self.scope.clone();
        self.env.func_sig(f, &scope, false)
    }

    /// A top-level alias using `typeof(expr)` gets its body where the file
    /// reaches it, with the expressions' types there (spec §2.2.7): later
    /// annotations, and the modules checked after this one, see it.
    fn typeof_alias(&mut self, a: TypeAlias<'_>) {
        // Top-level aliases only (function bodies have none of their own).
        let Some(module) = self.scope.module.filter(|_| self.fns.len() == 1) else {
            return;
        };
        let Some(def) = self
            .env
            .module_types
            .get(&module)
            .and_then(|t| t.get(a.name().text()))
            .copied()
        else {
            return;
        };
        if self.env.typeof_aliases.get(&def) != Some(&module) {
            return;
        }
        self.env.typeof_aliases.remove(&def);
        let mut exprs: Vec<Expr<'_>> = Vec::new();
        typeof_exprs(a.ty(), &mut exprs);
        for e in exprs {
            let ty = self.infer(e);
            let ty = self.env.types.widen(ty);
            self.env
                .typeof_types
                .insert((e.span().start, e.span().end), ty);
        }
        let scope = self.scope.clone();
        self.env.resolve_alias_again(def, a, &scope);
        self.env.typeof_resolved.insert(module);
        self.env.typeof_alias_names.insert(a.name().text().into());
    }

    /// The type of `...` in the current function.
    pub fn current_vararg(&self) -> Option<TypeId> {
        self.fns.last().and_then(|f| f.vararg)
    }

    /// The declared or base type of a local.
    pub fn var_base(&self, v: VarId) -> TypeId {
        self.vars[v.0 as usize].base
    }

    /// The type of a local in a flow. An unannotated local a closure
    /// assigns (or captured and assigned later) is, inside closures, the
    /// union of the values assigned to it: the previous pass's, or in the
    /// first pass what was assigned so far (spec §2.4.1).
    pub fn var_type_in(&self, v: VarId, flow: &Flow) -> TypeId {
        let var = &self.vars[v.0 as usize];
        if var.depth < self.depth && var.declared.is_none() && var.reassigned_in_closure {
            // What this function assigned or narrowed it to (a function
            // starts without the outer narrowing of these).
            if let Some(&t) = flow.get(&v) {
                return t;
            }
            self.closure_hit.set(true);
            return var
                .decl
                .and_then(|d| self.closure_seeds.get(&d).copied())
                .unwrap_or(var.assigned);
        }
        flow.get(&v).copied().unwrap_or(var.base)
    }

    /// A function is made here: the unannotated locals in scope that
    /// closures see as assigned (see [`Self::var_type_in`]) take their
    /// narrowed types now, and count what is assigned to them from here.
    fn note_captures(&mut self) {
        let visible: Vec<VarId> = self
            .scopes
            .stacks
            .iter()
            .filter_map(|s| s.last().copied())
            .filter(|v| {
                let var = &self.vars[v.0 as usize];
                var.declared.is_none() && var.reassigned_in_closure
            })
            .collect();
        for v in visible {
            // Where it is declared, a call may have dropped its narrowing to
            // `nil` (a callback may have set it): not known to be assigned.
            let here = self.vars[v.0 as usize].depth == self.depth;
            let now = if here {
                self.flow.get(&v).copied()
            } else {
                Some(self.var_type(v))
            };
            let var = &self.vars[v.0 as usize];
            // a forward declaration before its value: no `nil` of its own
            let now = match now {
                Some(t) if !(var.nil_init && t == Types::NIL) => Some(t),
                None if !var.nil_init => Some(var.base),
                _ => None,
            };
            let assigned = match now {
                Some(t) => self.env.types.union(&[var.assigned, t]),
                None => var.assigned,
            };
            let base = var.base;
            if assigned != self.vars[v.0 as usize].assigned {
                self.set_base(v, base, assigned);
            }
            self.vars[v.0 as usize].captured = true;
        }
    }

    /// The unions of the values assigned to the locals closures read as
    /// such (see [`Self::var_type_in`]), by declaration; `any` for one
    /// never assigned a value.
    pub(crate) fn closure_finals(&mut self) -> FxHashMap<u32, TypeId> {
        let mut out: FxHashMap<u32, TypeId> = FxHashMap::default();
        for var in &self.vars {
            let (Some(d), None, true) = (var.decl, var.declared, var.reassigned_in_closure) else {
                continue;
            };
            let t = match out.get(&d) {
                Some(&prev) => self.env.types.union(&[prev, var.assigned]),
                None => var.assigned,
            };
            out.insert(d, t);
        }
        for t in out.values_mut() {
            if *t == Types::NEVER {
                *t = Types::ANY;
            }
        }
        out
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
                let value = self.fill_empty_fields(value, var.base);
                let widened = self.widen_assigned(value, var.base);
                self.flow.insert(v, widened);
                // The base widens to cover every assigned value.
                let base = self.env.types.union(&[var.base, widened]);
                let assigned = if var.captured {
                    self.env.types.union(&[var.assigned, widened])
                } else {
                    var.assigned
                };
                self.set_base(v, base, assigned);
            }
        }
    }

    /// The type a value assigned to an unannotated local takes: widened as
    /// in `local`, except for a literal the local's type so far already has
    /// (`w = false` for a `T | false` local stays `false`), decision L61.
    fn widen_assigned(&mut self, value: TypeId, base: TypeId) -> TypeId {
        let have = self.env.types.members(base);
        let (kept, rest): (Vec<TypeId>, Vec<TypeId>) =
            self.env.types.members(value).into_iter().partition(|m| {
                have.contains(m)
                    && matches!(
                        self.env.types.get(*m),
                        Ty::Bool(_) | Ty::Num(_) | Ty::Str(_)
                    )
            });
        if kept.is_empty() {
            return self.env.types.widen_value(value);
        }
        let mut out = kept;
        if !rest.is_empty() {
            let rest = self.env.types.union(&rest);
            out.push(self.env.types.widen_value(rest));
        }
        self.env.types.union(&out)
    }

    /// A table assigned to a local without an annotation: where it fits the
    /// local's type so far, its empty `{}` fields take that type's field
    /// types (`p = { seen = {} }` for a `{ seen: { [number]: boolean } }`),
    /// decision L59. Its other fields stay as they are.
    fn fill_empty_fields(&mut self, value: TypeId, base: TypeId) -> TypeId {
        if !matches!(self.env.types.get(value), Ty::Record(_)) {
            return value;
        }
        let target = self.env.types.non_nil(base);
        if target == Types::ANY
            || target == Types::NEVER
            || !self.env.assignable(value, target, self.mode)
        {
            return value;
        }
        self.fill_fields(value, target, 0)
    }

    fn fill_fields(&mut self, value: TypeId, target: TypeId, depth: u32) -> TypeId {
        let Ty::Record(r) = self.env.types.get(value).clone() else {
            return value;
        };
        let mut fields = r.fields.clone();
        let mut changed = false;
        for (name, f) in r.fields.iter() {
            let Some((ft, _)) = self.env.field_of(target, name) else {
                continue;
            };
            let ft = self.env.types.non_nil(ft);
            let filled = match self.env.types.get(f.ty) {
                Ty::Record(inner) if inner.fields.is_empty() && inner.indexer.is_none() => {
                    (ft != Types::ANY && self.env.assignable(f.ty, ft, self.mode)).then_some(ft)
                }
                Ty::Record(_) if depth < 8 => {
                    let t = self.fill_fields(f.ty, ft, depth + 1);
                    (t != f.ty).then_some(t)
                }
                _ => None,
            };
            if let Some(ty) = filled {
                fields.insert(name.into(), Field { ty, ..*f });
                changed = true;
            }
        }
        if !changed {
            return value;
        }
        self.env.types.record(Record {
            fields,
            indexer: r.indexer,
            open: r.open,
            id: 0,
        })
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
        let ns = self.env.declared.global_type(super::env::ADDON_NAMESPACE);
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
        self.hoist(parse.view().block());
        self.block_stmts(parse.view().block());
        // Calls of methods defined further down, now that all are known.
        for (key, callee, use_) in std::mem::take(&mut self.pending_caps) {
            if let Some(gives) = self.cap_of(&key) {
                self.report_past(&callee, gives, &use_);
            }
        }
        // Importers read an export after the module's top level has run
        // (spec §5.5), so a table has the fields assigned to it since its
        // declaration, and it has escaped: it is sealed (spec §2.4.2).
        for (name, v) in std::mem::take(&mut self.export_vars) {
            if let Some(&cap) = self.result_caps.get(&v) {
                self.export_caps.insert(name.clone(), cap);
            }
            let t = self.var_type(v);
            let t = self.sealed(t);
            self.exports.insert(name, t);
        }
        // A module's file-level locals, for `import internal` (kept with
        // the exports, under `INTERNAL` + name).
        if self.is_module {
            for s in parse.view().block().stmts() {
                let names: Vec<&str> = match s.kind() {
                    StmtKind::Local { names, .. } => names.iter().map(|n| n.name.text()).collect(),
                    StmtKind::LocalFunction { name, .. } => vec![name.text()],
                    StmtKind::Function { name, .. }
                        if name.path().len() == 1 && name.method().is_none() =>
                    {
                        vec![name.path().at(0).text()]
                    }
                    StmtKind::Const { name, .. } => vec![name.text()],
                    StmtKind::Class(c) => vec![c.name().text()],
                    _ => Vec::new(),
                };
                for name in names {
                    if let Some(v) = self.lookup_var(name) {
                        if let Some(&cap) = self.result_caps.get(&v) {
                            self.export_caps
                                .insert(format!("{INTERNAL}{name}").into(), cap);
                        }
                        let t = self.var_type(v);
                        self.exports.insert(format!("{INTERNAL}{name}").into(), t);
                    }
                }
            }
            // The exports are file-level names too: `import internal`
            // reads them with their types (an `export function` was
            // missing there and read as `any`, decision L66).
            let exported: Vec<(Box<str>, TypeId)> = self
                .exports
                .iter()
                .filter(|(k, _)| !k.starts_with(INTERNAL))
                .map(|(k, &t)| (k.clone(), t))
                .collect();
            for (name, t) in exported {
                let key: Box<str> = format!("{INTERNAL}{name}").into();
                if let Some(&cap) = self.export_caps.get(&name) {
                    self.export_caps.entry(key.clone()).or_insert(cap);
                }
                self.exports.entry(key).or_insert(t);
            }
        }
        self.fns.pop();
    }

    /// Top-level classes and enums are hoisted (decision T76), and so are
    /// the top-level functions of a module (spec §5.1).
    fn hoist(&mut self, b: Block<'_>) {
        // The overload signatures before a function: callers see them.
        let mut overloads: Vec<(Box<str>, TypeId)> = Vec::new();
        for s in b.stmts() {
            let kind = match s.kind() {
                StmtKind::Export(inner) => inner.kind(),
                k => k,
            };
            match kind {
                StmtKind::Overload { name, func } => {
                    let t = self.overload_signature(func);
                    overloads.push((name.text().into(), t));
                    continue;
                }
                StmtKind::Function { name, func }
                    if self.is_module && name.path().len() == 1 && name.method().is_none() =>
                {
                    let n = name.path().at(0).text();
                    let t = match overload_set_of(&mut self.env.types, &overloads, n) {
                        Some(set) => set,
                        None => {
                            let sig = self.func_sig(&FnSig::of(func));
                            self.env.types.func(sig)
                        }
                    };
                    self.declare(n, Some(t), t, false, None);
                }
                StmtKind::Class(c) => {
                    if let Some(def) = self.env.lookup_type(c.name().text(), &self.scope) {
                        let t = self.env.types.intern(Ty::ClassOf(def));
                        self.declare(c.name().text(), Some(t), t, true, None);
                    }
                }
                StmtKind::Enum(e) => {
                    if let Some(def) = self.env.lookup_type(e.name().text(), &self.scope) {
                        let t = self.env.types.intern(Ty::ClassOf(def));
                        self.declare(e.name().text(), Some(t), t, true, None);
                    }
                }
                _ => {}
            }
            overloads.clear();
        }
    }

    /// The overload set the pending `overload function` signatures of
    /// `name` make, now that its implementation (`actual`) is here: each
    /// one the implementation must accept (E3044).
    fn take_overloads(&mut self, name: &str, actual: TypeId) -> Option<TypeId> {
        if !self
            .pending_overloads
            .iter()
            .any(|(n, _, _)| n.as_ref() == name)
        {
            return None;
        }
        let pending = std::mem::take(&mut self.pending_overloads);
        let mut set: Option<TypeId> = None;
        for (n, sig, span) in pending {
            if n.as_ref() != name {
                self.report(
                    "E3047",
                    span,
                    format!(
                        "the overload signature of `{n}` is not followed by its implementation"
                    ),
                );
                continue;
            }
            if !self.env.overload_accepted(actual, sig) {
                let msg = format!(
                    "the implementation of `{name}` (`{}`) does not accept this overload (`{}`)",
                    self.env.display(actual),
                    self.env.display(sig)
                );
                self.report("E3044", span, msg);
            }
            set = Some(match set {
                Some(s) => super::env::overload_set(&mut self.env.types, s, sig),
                None => sig,
            });
        }
        set
    }

    /// An `overload function` signature's type: without a body, no
    /// results unless it names them.
    fn overload_signature(&mut self, func: FuncNode<'_>) -> TypeId {
        let mut sig = self.func_sig(&FnSig::of(func));
        if func.ret().is_none() {
            sig.ret = Pack::empty();
        }
        self.env.types.func(sig)
    }

    /// Overload signatures left without their implementation (E3047).
    fn unimplemented_overloads(&mut self) {
        for (n, _, span) in std::mem::take(&mut self.pending_overloads) {
            self.report(
                "E3047",
                span,
                format!("the overload signature of `{n}` is not followed by its implementation"),
            );
        }
    }

    pub fn block(&mut self, b: Block<'_>) {
        self.push_scope();
        self.block_stmts(b);
        self.pop_scope();
    }

    /// A branch no value reaches (spec §2.2.8): checked for its effects on
    /// the file's types, with nothing reported. Whether the code after it
    /// runs still follows `return`, `break` and `error` only.
    fn unreachable_block(&mut self, b: Block<'_>) {
        let (own, env) = (self.diagnostics.len(), self.env.diagnostics.len());
        self.block(b);
        self.diagnostics.truncate(own);
        self.env.diagnostics.truncate(env);
    }

    fn block_stmts(&mut self, b: Block<'_>) {
        let outer = self.in_dead;
        for s in b.stmts() {
            // The first statement after a `return`, `break`, `continue` or
            // a call that never returns (W1005), once per region.
            if !self.live && !self.in_dead {
                self.report("W1005", s.span(), "unreachable code");
                self.in_dead = true;
            }
            // Overload signatures belong to the function right after them.
            let implements = match s.kind() {
                StmtKind::Export(inner) => inner,
                _ => s,
            };
            let continues = match implements.kind() {
                StmtKind::Overload { .. } => true,
                StmtKind::LocalFunction { name, .. } => self
                    .pending_overloads
                    .iter()
                    .all(|(n, _, _)| n.as_ref() == name.text()),
                StmtKind::Function { name, .. } => {
                    name.path().len() == 1
                        && name.method().is_none()
                        && self
                            .pending_overloads
                            .iter()
                            .all(|(n, _, _)| n.as_ref() == name.path().at(0).text())
                }
                _ => false,
            };
            if !continues {
                self.unimplemented_overloads();
            }
            self.stmt(s);
        }
        self.unimplemented_overloads();
        self.in_dead = outer;
    }

    fn stmt(&mut self, s: Stmt<'_>) {
        self.next_handler = self.decorated_handler(s.decorators());
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.stmt_inner(s));
        self.next_handler = None;
        if !s.decorators().is_empty() {
            self.check_cases(s);
        }
    }

    /// Whether `name` names a type here: a built-in one, a type
    /// parameter, the module's or an imported type, or a declared one.
    fn names_type(&self, name: &str) -> bool {
        matches!(
            name,
            "any"
                | "unknown"
                | "never"
                | "boolean"
                | "number"
                | "string"
                | "table"
                | "thread"
                | "userdata"
                | "nil"
        ) || self.scope.params.iter().any(|(n, _)| n.as_ref() == name)
            || self.env.lookup_type(name, &self.scope).is_some()
    }

    /// `@cases(list)` of `wowlua:test` (decision T215): every case fits the
    /// test function's parameters (E8120).
    fn check_cases(&mut self, s: Stmt<'_>) {
        let func = match s.kind() {
            StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => func,
            _ => return,
        };
        let Some(d) = s.decorators().iter().find(|d| {
            let path = d.path();
            let id = if let Some([n]) = path.exactly::<1>() {
                self.intrinsic_path(n.text(), None)
            } else if let Some([m, n]) = path.exactly::<2>() {
                self.intrinsic_path(m.text(), Some(n.text()))
            } else {
                None
            };
            id.as_deref() == Some("test.cases")
        }) else {
            return;
        };
        let Some(ExprKind::Table(entries)) =
            d.args().and_then(|a| a.exprs().first().map(|e| e.kind()))
        else {
            return;
        };
        let sig = self.func_sig(&FnSig::of(func));
        for (n, entry) in entries.iter().enumerate() {
            let TableField::Positional(case) = entry else {
                continue;
            };
            // A table literal holds the arguments; another value is the only one.
            let values: Vec<Expr<'_>> = match case.kind() {
                ExprKind::Table(fields) => fields
                    .iter()
                    .filter_map(|f| match f {
                        TableField::Positional(v) => Some(v),
                        _ => None,
                    })
                    .collect(),
                _ => vec![case],
            };
            let n = n + 1;
            if values.len() > sig.params.len() && sig.rest.is_none() {
                let msg = format!(
                    "case {n} has {} values; the test takes {}",
                    values.len(),
                    sig.params.len()
                );
                self.report("E8120", case.span(), msg);
                continue;
            }
            if let Some(p) = sig.params.iter().skip(values.len()).find(|p| !p.optional) {
                let msg = format!(
                    "case {n} has no value for `{}`",
                    p.name.as_deref().unwrap_or("?")
                );
                self.report("E8120", case.span(), msg);
                continue;
            }
            for (i, v) in values.iter().enumerate() {
                let Some(p) = sig.params.get(i) else { break };
                // Typed by the parameter, as an argument is (a literal
                // field fits a literal union: `{ state = "idle" }`).
                let t = self.check(*v, Some(p.ty));
                if !self.env.assignable(t, p.ty, self.mode) {
                    let msg = format!(
                        "case {n}: `{}` is not assignable to `{}` (parameter `{}`)",
                        self.env.display(t),
                        self.env.display(p.ty),
                        p.name.as_deref().unwrap_or("?")
                    );
                    self.report("E8120", v.span(), msg);
                }
            }
        }
    }

    /// The standard-library `@intrinsic` id a callee (`on`, `events.on`)
    /// names through an import that no local shadows.
    /// A member of an intrinsic is named by the intrinsic's id and the
    /// member (`FramePool.new` → `pool.FramePool.new`).
    pub(crate) fn intrinsic_of(&self, callee: Expr<'_>) -> Option<String> {
        match callee.kind() {
            ExprKind::Name(n) => self.intrinsic_path(n.text(), None),
            ExprKind::Field { obj, name } => {
                if let ExprKind::Name(m) = obj.kind()
                    && let Some(id) = self.intrinsic_path(m.text(), Some(name.text()))
                {
                    return Some(id);
                }
                self.intrinsic_of(obj)
                    .map(|id| format!("{id}.{}", name.text()))
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
    fn decorated_handler(&mut self, decorators: Decorators<'_>) -> Option<(Func, String)> {
        for d in decorators {
            let path = d.path();
            let id = match path.len() {
                1 => self.intrinsic_path(path.at(0).text(), None),
                2 => self.intrinsic_path(path.at(0).text(), Some(path.at(1).text())),
                _ => None,
            };
            let Some(id) = id else { continue };
            let args = d.args().as_ref().map(|a| a.exprs()).unwrap_or_default();
            let names: Vec<String> = args.iter().map_while(|e| literal_string(e)).collect();
            let (skip, event_names) = match id.as_str() {
                "events.event" => (0, args.len()),
                "events.unitEvent" => (0, 1),
                "events.bucket" => (1, args.len()),
                _ => (0, 0),
            };
            for e in args.iter().skip(skip).take(event_names) {
                if let Some(n) = literal_string(e) {
                    self.check_event_available(&n, e.span());
                }
            }
            if id.starts_with("hooks.") {
                self.check_hook_target(&id, args, true);
            }
            match id.as_str() {
                "events.event" if names.len() == 1 => {
                    let f = self.env.declared.handler(names[0].as_str())?.clone();
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
                        outer: Vec::new(),
                    };
                    return Some((f, format!("`{}`", names.join("`, `"))));
                }
                "events.unitEvent" if !names.is_empty() => {
                    let f = self.env.declared.handler(names[0].as_str())?.clone();
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

    fn stmt_inner(&mut self, s: Stmt<'_>) {
        match s.kind() {
            StmtKind::Local { names, values } => {
                // `local a, b: any, any = …` declares a third local `any`:
                // a name after an annotated one that names a type is a
                // misplaced annotation list (E2060).
                let mut annotated = false;
                for n in names.iter() {
                    let name = n.name.text();
                    if n.ty.is_some() {
                        annotated = true;
                    } else if annotated && self.names_type(name) {
                        let msg = format!(
                            "`{name}` is declared as a local, but it names a type: annotate each name (`local a: T, b: U`)"
                        );
                        self.report("E2060", n.name.span(), msg);
                    }
                }
                let expected: Vec<Option<TypeId>> = names
                    .iter()
                    .map(|n| n.ty.as_ref().map(|t| self.annotation(*t)))
                    .collect();
                // Fresh literals widen (`local s = "a"` is a string); values
                // typed elsewhere (a call's declared result, a variable,
                // a field, a `::` assertion) keep their type, unions of
                // literals included (decision L64).
                let keep: Vec<bool> = values.iter().map(typed_elsewhere).collect();
                let spread: Vec<(Span, String)> = names
                    .iter()
                    .map(|n| (n.name.span(), n.name.text().to_string()))
                    .collect();
                let values = self.expr_list_into(values, &expected, &spread);
                let no_values = values.is_empty();
                for (i, n) in names.iter().enumerate() {
                    let value = values.get(i).copied().unwrap_or(Types::NIL);
                    let declared = expected[i];
                    // `local x: T` without a value: `nil` until assigned,
                    // checked where it is read.
                    if let Some(d) = declared
                        && !no_values
                    {
                        let span = n.name.span();
                        self.check_assignable(span, value, d);
                    }
                    // W2063 (off unless `check.lints` turns it on): a local
                    // whose value is `any`.
                    if declared.is_none()
                        && !no_values
                        && value == Types::ANY
                        && !n.name.text().starts_with('_')
                    {
                        let msg = format!(
                            "`{}` is `any`: its value has no type (annotate it, or type where it comes from)",
                            n.name.text()
                        );
                        self.report("W2063", n.name.span(), msg);
                    }
                    let id =
                        self.declare(n.name.text(), declared, value, false, Some(n.name.span()));
                    // The names past the last value take that call's other
                    // results (`local ok, r = pcall(f)`).
                    let kept = keep.get(i).or(keep.last()).copied().unwrap_or(false);
                    if declared.is_none() && kept {
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
                let spread: Vec<(Span, String)> = targets
                    .iter()
                    .map(|t| {
                        let name = match t.kind() {
                            ExprKind::Name(n) => n.text().to_string(),
                            _ => "the target".to_string(),
                        };
                        (t.span(), name)
                    })
                    .collect();
                let values = self.expr_list_into(values, &expected, &spread);
                // A local function assigned again may hold another
                // function: its count no longer holds (W2062).
                for t in targets.iter() {
                    match t.kind() {
                        ExprKind::Name(n) => {
                            if let Some(var) = self.lookup_var(n.text()) {
                                self.result_caps.remove(&var);
                            }
                        }
                        // a table's method set again (`M.f = g`, `self.f = g`)
                        ExprKind::Field { obj, name } => {
                            if let Some(table) = self.cap_table(obj) {
                                self.method_caps.remove(&(table, name.text().into()));
                            }
                        }
                        _ => {}
                    }
                }
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
                let result = self.compound_result(op, current, v, s.span());
                self.assign_target(target, result);
            }
            StmtKind::Call(e) => {
                let results = self.infer_multi(e);
                // `error(...)` and other functions returning `never`.
                if results.types.first() == Some(&Types::NEVER) {
                    self.live = false;
                }
                // `assert(cond)` narrows like `if cond` (spec §2.5).
                if let ExprKind::Call { callee, args } = e.kind()
                    && matches!(callee.kind(), ExprKind::Name(n) if n.text() == "assert" && self.lookup_var("assert").is_none())
                    && let Some(cond) = args.exprs().first()
                {
                    let (t_flow, _) = self.narrow(cond, &self.flow.clone());
                    self.flow = t_flow;
                }
            }
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => self.check_loop(body, |c, keep| {
                c.mark_probes(cond);
                c.infer(cond);
                c.forget_assigned(body, keep);
                let (t_flow, f_flow) = c.narrow(cond, &c.flow.clone());
                c.flow = t_flow;
                c.block(body);
                let back = c.live.then(|| c.flow.clone());
                // `join` reads no flow but its arguments.
                let after_body = std::mem::take(&mut c.flow);
                let joined = c.join(&f_flow, &after_body, true, c.live);
                c.flow = joined;
                c.live = true;
                back
            }),
            StmtKind::Repeat { body, cond } => self.check_loop(body, |c, keep| {
                c.forget_assigned(body, keep);
                c.push_scope();
                c.block_stmts(body);
                c.mark_probes(cond);
                c.infer(cond);
                let (_, f) = c.narrow(cond, &c.flow.clone());
                let back = c.live.then_some(f);
                c.pop_scope();
                c.live = true;
                back
            }),
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let mut clauses: Vec<(Expr<'_>, Block<'_>)> = vec![(cond, then_block)];
                clauses.extend(elseifs.iter());
                let mut ends: Vec<(Flow, bool, super::flavors::Guard)> = Vec::new();
                // A type predicate that leaves its argument `never` makes a
                // branch unreachable (spec §2.2.8): from then on, every
                // later clause and the `else` too.
                let mut rest_unreachable = false;
                for (c, b) in clauses {
                    self.mark_probes(c);
                    self.infer(c);
                    let (t_flow, f_flow) = self.narrow(c, &self.flow.clone());
                    let (t_guard, f_guard) = self.narrow_guard(c, &self.guard.clone());
                    let unreachable = rest_unreachable || self.excluded_by_predicate(c, &t_flow);
                    rest_unreachable = rest_unreachable || self.excluded_by_predicate(c, &f_flow);
                    let entry_live = self.live;
                    self.flow = t_flow;
                    self.guard = t_guard;
                    if unreachable {
                        self.unreachable_block(b);
                    } else {
                        self.block(b);
                    }
                    ends.push((self.flow.clone(), self.live, self.guard.clone()));
                    self.flow = f_flow;
                    self.guard = f_guard;
                    self.live = entry_live;
                }
                match else_block {
                    Some(b) if rest_unreachable => {
                        self.unreachable_block(b);
                        ends.push((self.flow.clone(), self.live, self.guard.clone()));
                    }
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
                for e in [Some(start), Some(limit), step].into_iter().flatten() {
                    let t = self.infer(e);
                    self.check_assignable(e.span(), t, Types::NUMBER);
                }
                self.check_loop(body, |c, keep| {
                    c.forget_assigned(body, keep);
                    c.push_scope();
                    c.declare(
                        var.name.text(),
                        Some(Types::NUMBER),
                        Types::NUMBER,
                        false,
                        Some(var.name.span()),
                    );
                    let entry = c.flow.clone();
                    c.block(body);
                    c.pop_scope();
                    let back = c.live.then(|| c.flow.clone());
                    c.flow = c.join(&entry, &c.flow.clone(), true, c.live);
                    c.live = true;
                    back
                });
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                let types = self.expr_list(exprs, &[]);
                let loop_types = self
                    .iteration_types(&types, exprs.first().map(|e| e.span()).unwrap_or(s.span()));
                self.check_loop(body, |c, keep| {
                    c.forget_assigned(body, keep);
                    c.push_scope();
                    for (i, v) in vars.iter().enumerate() {
                        let declared = v.ty.as_ref().map(|t| c.annotation(*t));
                        let t = loop_types.get(i).copied().unwrap_or(Types::ANY);
                        c.declare_with(
                            v.name.text(),
                            declared,
                            t,
                            false,
                            Some(v.name.span()),
                            false,
                        );
                    }
                    let entry = c.flow.clone();
                    c.block(body);
                    c.pop_scope();
                    let back = c.live.then(|| c.flow.clone());
                    c.flow = c.join(&entry, &c.flow.clone(), true, c.live);
                    c.live = true;
                    back
                });
            }
            StmtKind::Function { name, func } => self.function_stmt(name, func),
            StmtKind::Overload { name, func } => {
                let t = self.overload_signature(func);
                self.pending_overloads
                    .push((name.text().into(), t, name.span()));
            }
            StmtKind::LocalFunction { name, func } => {
                let sig = self.func_sig(&FnSig::of(func));
                let impl_t = self.env.types.func(sig);
                // Callers (and the body, recursing) see the overloads.
                let set = self.take_overloads(name.text(), impl_t);
                let t = set.unwrap_or(impl_t);
                let id = self.declare(name.text(), None, t, false, Some(name.span()));
                let handler = self.next_handler.take();
                let actual = self.function_body(
                    &FnSig::of(func),
                    handler.as_ref().map(|h| h.0.clone()),
                    None,
                );
                if let Some(h) = &handler {
                    self.check_handler(actual, h, name.span());
                }
                if let Some(cap) = self.last_result_cap.take() {
                    self.result_caps.insert(id, cap);
                }
                let actual = set.unwrap_or(actual);
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
                    self.check_pack(values, &pack, &exp, s.span());
                }
                if let Some(f) = self.fns.last_mut() {
                    f.returns.push(pack);
                }
                self.live = false;
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.live
                    && let Some(l) = self.loops.last_mut()
                {
                    let exits = if matches!(s.kind(), StmtKind::Break) {
                        &mut l.breaks
                    } else {
                        &mut l.continues
                    };
                    exits.push(self.flow.clone());
                }
                self.live = false;
            }
            StmtKind::LocalDestructure {
                fields, value, ty, ..
            } => {
                let declared = ty.as_ref().map(|t| self.annotation(*t));
                let source = match value {
                    Some(v) => self.check(v, declared),
                    None => Types::ANY,
                };
                let source = declared.unwrap_or(source);
                for f in fields {
                    let field_ty = self
                        .member_type(source, f.name.text(), f.name.span(), false)
                        .unwrap_or(Types::ANY);
                    let t = match &f.default {
                        Some(d) => {
                            let dt = self.infer(*d);
                            let non_nil = self.env.types.non_nil(field_ty);
                            self.env.types.union(&[non_nil, dt])
                        }
                        None => field_ty,
                    };
                    self.declare(f.name.text(), None, t, false, Some(f.name.span()));
                }
            }
            StmtKind::Const { name, ty, value } => {
                let declared = ty.as_ref().map(|t| self.annotation(*t));
                let t = match value {
                    Some(v) => self.check(v, declared),
                    None => declared.unwrap_or(Types::ANY),
                };
                if let (Some(d), Some(v)) = (declared, value) {
                    self.check_assignable(v.span(), t, d);
                }
                // `const` keeps literal types (spec §2.4). An annotated one
                // narrows as an annotated `local` does: to the annotation's
                // literals and members, not to a table literal's own type.
                let id = self.declare(
                    name.text(),
                    Some(declared.unwrap_or(t)),
                    t,
                    true,
                    Some(name.span()),
                );
                let narrowed = match declared {
                    Some(d)
                        if d != Types::ANY
                            && t != Types::ANY
                            && self.env.assignable(t, d, self.mode) =>
                    {
                        self.narrow_to_assigned(d, t)
                    }
                    Some(d) => d,
                    None => t,
                };
                self.flow.insert(id, narrowed);
            }
            StmtKind::Class(c) => self.class_body(c),
            StmtKind::TypeAlias(a) => self.typeof_alias(a),
            StmtKind::Enum(_) | StmtKind::Interface(_) | StmtKind::Declare(_) => {}
            StmtKind::Import(_) => {}
            StmtKind::Export(inner) => {
                self.stmt(inner);
                for name in exported_names(inner) {
                    if let Some(v) = self.lookup_var(&name) {
                        self.export_vars.push((name.into(), v));
                    }
                }
            }
            StmtKind::Error => {}
        }
    }

    /// The assignment target is a global or lies in one (`X`, `_G.X`,
    /// `X.a.b`, `X[k]`): what it stores is reachable from other addons.
    pub(crate) fn global_rooted(&self, target: Expr<'_>) -> bool {
        let mut e = target;
        loop {
            match e.kind() {
                ExprKind::Name(n) => return self.lookup_var(n.text()).is_none(),
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

    /// Checks a loop. `run` checks it with the locals assigned in its body
    /// forgetting their narrowing at the loop start, except `keep`, and
    /// gives the flow that goes back to the start at the body's end.
    ///
    /// Declared locals the loop entry narrows (`n` after `n = 0`) are
    /// first tried with that narrowing kept: the trial holds when every
    /// path back to the loop start (the body's end, `continue`) leaves them
    /// within it, and their `break` paths then join the flow after the
    /// loop. Unannotated locals are tried with their type before the loop:
    /// the trial holds when the paths back bring no new type, and is tried
    /// once more with the types they brought (decision L65). Otherwise the
    /// loop is checked again with them forgotten.
    fn check_loop(
        &mut self,
        body: Block<'_>,
        mut run: impl FnMut(&mut Self, &[VarId]) -> Option<Flow>,
    ) {
        let mut keep = self.loop_kept(body);
        // The unannotated locals' types at the loop start, once a trial
        // found the paths back bringing more.
        let mut grown: FxHashMap<VarId, TypeId> = FxHashMap::default();
        for _round in 0..2 {
            if keep.is_empty() {
                break;
            }
            let trial = LoopTrial {
                flow: self.flow.clone(),
                field_vars: self.field_vars.clone(),
                live: self.live,
                in_dead: self.in_dead,
                guard: self.guard.clone(),
                vars: self.vars.len(),
                diagnostics: self.diagnostics.len(),
                env_diagnostics: self.env.diagnostics.len(),
                returns: self.fns.last().map_or(0, |f| f.returns.len()),
                base_log: self.base_log.replace(Vec::new()),
            };
            // The kept narrowing, as the declared members it fits (`0`
            // in a `number?` is tried as `number`).
            let mut entry = Vec::new();
            for &v in &keep {
                let now = self.var_type(v);
                let t = match self.vars[v.0 as usize].declared {
                    Some(declared) => self.loosen(now, declared),
                    None => grown.get(&v).copied().unwrap_or(now),
                };
                self.flow.insert(v, t);
                entry.push((v, t));
            }
            self.loops.push(LoopFlows::default());
            let back = run(self, &keep);
            let exits = self.loops.pop().unwrap_or_default();
            // The declared locals that left their narrowing, and the
            // unannotated ones the paths back brought new types to.
            let mut failed: Vec<VarId> = Vec::new();
            let mut more: Vec<(VarId, TypeId)> = Vec::new();
            for &(v, t) in &entry {
                let declared = self.vars[v.0 as usize].declared.is_some();
                let mut u = t;
                for f in back.iter().chain(&exits.continues) {
                    let now = self.var_type_in(v, f);
                    if declared {
                        if !self.env.assignable(now, t, Mode::Strict) {
                            failed.push(v);
                            break;
                        }
                    } else {
                        u = self.env.types.union(&[u, now]);
                    }
                }
                if u != t {
                    more.push((v, u));
                }
            }
            let log = std::mem::replace(&mut self.base_log, trial.base_log);
            if failed.is_empty() && more.is_empty() {
                if let (Some(outer), Some(log)) = (&mut self.base_log, log) {
                    outer.extend(log);
                }
                for b in &exits.breaks {
                    for &(v, _) in &entry {
                        let after = self.var_type(v);
                        let left = self.var_type_in(v, b);
                        if !self.env.assignable(left, after, Mode::Strict) {
                            let u = self.env.types.union(&[after, left]);
                            self.flow.insert(v, u);
                        }
                    }
                }
                return;
            }
            for (v, base, assigned) in log.unwrap_or_default().into_iter().rev() {
                if let Some(var) = self.vars.get_mut(v.0 as usize) {
                    var.base = base;
                    var.assigned = assigned;
                }
            }
            self.flow = trial.flow;
            self.field_vars = trial.field_vars;
            self.live = trial.live;
            self.in_dead = trial.in_dead;
            self.guard = trial.guard;
            self.vars.truncate(trial.vars);
            self.diagnostics.truncate(trial.diagnostics);
            self.env.diagnostics.truncate(trial.env_diagnostics);
            if let Some(f) = self.fns.last_mut() {
                f.returns.truncate(trial.returns);
            }
            // A second round: without the declared locals that left their
            // narrowing, the unannotated ones with what the paths back
            // brought. A second round that still finds more gives up.
            if !grown.is_empty() {
                break;
            }
            keep.retain(|v| !failed.contains(v));
            grown.extend(more);
        }
        self.loops.push(LoopFlows::default());
        run(self, &[]);
        self.loops.pop();
    }

    /// The members of `declared` that a member of `t` fits, or `t` where
    /// one fits none.
    fn loosen(&mut self, t: TypeId, declared: TypeId) -> TypeId {
        let members = self.env.types.members(t);
        let out: Vec<TypeId> = self
            .env
            .types
            .members(declared)
            .into_iter()
            .filter(|&d| {
                members
                    .iter()
                    .any(|&m| self.env.assignable(m, d, Mode::Strict))
            })
            .collect();
        let covered = members
            .iter()
            .all(|&m| out.iter().any(|&d| self.env.assignable(m, d, Mode::Strict)));
        if out.is_empty() || !covered {
            return t;
        }
        self.env.types.union(&out)
    }

    /// Replaces a local's base type (logged while a loop is on trial).
    fn set_base(&mut self, v: VarId, t: TypeId, assigned: TypeId) {
        let var = &mut self.vars[v.0 as usize];
        if let Some(log) = &mut self.base_log {
            log.push((v, var.base, var.assigned));
        }
        var.base = t;
        var.assigned = assigned;
    }

    /// The locals a loop body assigns that keep a type at the loop start:
    /// the declared ones the loop entry narrows, and the unannotated ones
    /// (a closure assigning one may run at any time: those are left out).
    fn loop_kept(&self, body: Block<'_>) -> Vec<VarId> {
        let mut out: Vec<VarId> = Self::assigned_names(body)
            .into_iter()
            .filter_map(|name| self.lookup_var(name))
            .filter(|&v| {
                let var = &self.vars[v.0 as usize];
                if var.assigned_in_closure || var.depth != self.depth {
                    return false;
                }
                match var.declared {
                    Some(d) => d != Types::ANY && self.flow.get(&v).is_some_and(|&t| t != d),
                    None => self.var_type(v) != Types::ANY,
                }
            })
            .collect();
        out.sort();
        out
    }

    /// The names a loop body assigns (`x = …`, `x += …`).
    fn assigned_names<'a>(body: Block<'a>) -> FxHashSet<&'a str> {
        let mut names: FxHashSet<&str> = FxHashSet::default();
        wowlua_syntax::visit::walk_stmts(body, &mut |s| match s.kind() {
            StmtKind::Assign { targets, .. } => {
                for t in targets {
                    if let ExprKind::Name(n) = t.kind() {
                        names.insert(n.text());
                    }
                }
            }
            StmtKind::Compound { target, .. } => {
                if let ExprKind::Name(n) = target.kind() {
                    names.insert(n.text());
                }
            }
            _ => {}
        });
        names
    }

    /// Locals assigned in a loop body lose their narrowing at the loop
    /// start (the body may run again), except `keep`.
    fn forget_assigned(&mut self, body: Block<'_>, keep: &[VarId]) {
        for name in Self::assigned_names(body) {
            if let Some(v) = self.lookup_var(name)
                && !keep.contains(&v)
            {
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
    fn function_stmt(&mut self, name: FuncName<'_>, func: FuncNode<'_>) {
        let method = name.method().is_some();
        // The table the function is stored in gives `self`'s type.
        let mut owner: Option<TypeId> = None;
        if name.path().len() > 1 || method {
            // `t.a.f` is stored in `t.a`; `t.a:m` in `t.a` too.
            let path = name.path();
            let end = if method { path.len() } else { path.len() - 1 };
            let mut t = self.name_type(path.at(0));
            for n in path.iter().take(end).skip(1) {
                t = self
                    .member_type(t, n.text(), n.span(), false)
                    .unwrap_or(Types::ANY);
            }
            owner = Some(t);
        }
        let expected = match (&owner, name.path().len(), &name.method()) {
            (Some(o), _, Some(m)) => self.member_type(*o, m.text(), m.span(), true),
            (Some(o), n, None) if n > 1 => self.member_type(
                *o,
                name.path().at(n - 1).text(),
                name.path().at(n - 1).span(),
                true,
            ),
            _ => None,
        };
        let self_ty = if method { owner } else { None };
        // A method of a file-level table: its count by table and name.
        let table = (name.path().len() == if method { 1 } else { 2 })
            .then(|| self.lookup_var(name.path().at(0).text()))
            .flatten();
        let method_name: Option<Box<str>> = table.map(|_| {
            name.method()
                .unwrap_or_else(|| name.path().at(1))
                .text()
                .into()
        });
        if method {
            self.pending_self_owner = table;
        }
        let handler = self.next_handler.take();
        let expected = expected
            .and_then(|e| self.as_func(e))
            .or_else(|| handler.as_ref().map(|h| h.0.clone()));
        // Its overload signatures, taken before the body is checked (a
        // block reports those left pending at its end).
        let overloads = (name.path().len() == 1 && !method)
            .then(|| {
                let sig = self.func_sig(&FnSig::of(func));
                let declared = self.env.types.func(sig);
                self.take_overloads(name.path().at(0).text(), declared)
            })
            .flatten();
        let t = self.function_body(&FnSig::of(func), expected, self_ty);
        let cap = self.last_result_cap.take();
        if let (Some(v), Some(m)) = (table, method_name) {
            match cap {
                Some(c) => self.method_caps.insert((v, m), c),
                None => self.method_caps.remove(&(v, m)),
            };
        } else if name.path().len() == 1
            && !method
            && let Some(v) = self.lookup_var(name.path().at(0).text())
        {
            // (a module's `function f()` is a file-level local)
            match cap {
                Some(c) => self.result_caps.insert(v, c),
                None => self.result_caps.remove(&v),
            };
        }
        if let Some(h) = &handler {
            let span = name
                .method()
                .as_ref()
                .unwrap_or(&name.path().at(name.path().len() - 1))
                .span();
            self.check_handler(t, h, span);
        }
        if name.path().len() == 1 && !method {
            let n = &name.path().at(0);
            // Callers see the overloads.
            let t = overloads.unwrap_or(t);
            match self.lookup_var(n.text()) {
                Some(v) => self.assign_var(v, t, n.span()),
                None => self.assign_global(*n, t),
            }
        } else if let Some(o) = owner {
            let last = name
                .method()
                .unwrap_or_else(|| name.path().last().expect("a name"));
            // A method stored in a table takes `self` first.
            let stored = if method { self.env.with_self(t, o) } else { t };
            self.extend_open(o, last.text(), stored);
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
        f: &FnSig<'_>,
        expected: Option<Func>,
        self_ty: Option<TypeId>,
    ) -> TypeId {
        let mut sig = self.func_sig(f);
        // Strict mode: parameters of non-callback functions need
        // annotations (E2050).
        let callback = std::mem::take(&mut self.callback_next);
        if self.mode == Mode::Strict && expected.is_none() && !callback {
            for p in f
                .params
                .iter()
                .filter(|p| p.ty.is_none() && p.default.is_none())
            {
                let msg = format!("parameter `{}` needs a type annotation", p.name.text());
                self.report("E2050", p.name.span(), msg);
            }
        }
        // Contextual parameter types for unannotated parameters.
        if let Some(exp) = &expected {
            for (i, p) in sig.params.iter_mut().enumerate() {
                if f.params[i].ty.is_none() {
                    // Past the expected parameters, its rest's type:
                    // `(...string) -> ()` types each (decision L78).
                    if let Some(ep) = exp.params.get(i) {
                        p.ty = ep.ty;
                    } else if let Some(Rest::Variadic(t)) = exp.rest {
                        p.ty = t;
                    }
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
        self.note_captures();
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
        let owner = self.pending_self_owner.take();
        if let Some(st) = self_ty {
            let id = self.declare("self", Some(st), st, false, None);
            if let Some(o) = owner {
                self.self_owners.insert(id, o);
            }
        }
        for (i, (p, sp)) in f.params.iter().zip(&sig.params).enumerate() {
            let declared = p.ty.map(|_| sp.ty);
            let t = match p.default {
                Some(d) => {
                    // In Lua every parameter is in scope in a default: this
                    // one and the later ones are there, not set yet, so a
                    // name of theirs is theirs, not an outer one (E1004).
                    self.push_scope();
                    let mut unset = Vec::new();
                    for (q, sq) in f.params.iter().zip(&sig.params).skip(i) {
                        let id = self.declare(q.name.text(), Some(sq.ty), sq.ty, false, None);
                        self.unset_params.insert(id);
                        unset.push(id);
                    }
                    let dt = self.check(d, Some(sp.ty));
                    self.pop_scope();
                    for id in unset {
                        self.unset_params.remove(&id);
                    }
                    let non_nil = self.env.types.non_nil(sp.ty);
                    if p.ty.is_some() {
                        non_nil
                    } else {
                        self.env.types.union(&[non_nil, dt])
                    }
                }
                None => sp.ty,
            };
            // W2063 (off unless `check.lints` turns it on): an unannotated
            // parameter that nothing gives a type.
            if p.ty.is_none() && t == Types::ANY && !p.name.text().starts_with('_') {
                let msg = format!(
                    "parameter `{}` is `any`: nothing gives it a type (annotate it)",
                    p.name.text()
                );
                self.report("W2063", p.name.span(), msg);
            }
            // Runtime checks of annotated parameters (spec §7.5), after
            // defaults: a parameter with a default is checked against `T`.
            if p.ty.is_some()
                && let Some(c) = self.value_check(t)
            {
                self.value_checks.insert(p.name.span(), c);
            }
            let id = self.declare(
                p.name.text(),
                declared.or(Some(sp.ty)),
                t,
                false,
                Some(p.name.span()),
            );
            self.flow.insert(id, t);
        }
        let vararg = sig.rest.as_ref().map(|r| match r {
            Rest::Variadic(t) => *t,
            Rest::Pack(_) => Types::ANY,
        });
        if let (Some(v), Some(t)) = (f.vararg, vararg)
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
        self.block_stmts(f.body);
        let ctx = self.fns.pop().expect("function");
        // The most values it can give: each `return` a fixed number (none
        // ends in `...` or a call of open length), else unknown (W2062).
        self.last_result_cap = ctx
            .returns
            .iter()
            .all(|p| p.rest.is_none())
            .then(|| ctx.returns.iter().map(|p| p.types.len()).max().unwrap_or(0));
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
            types.push(self.env.types.widen_value(u));
        }
        let rest = packs.iter().find_map(|p| p.rest.clone());
        Pack { types, rest }
    }

    /// Checks returned values against the annotated results.
    fn check_pack(&mut self, values: Exprs<'_>, got: &Pack, expected: &Pack, span: Span) {
        for (i, &want) in expected.types.iter().enumerate() {
            let have = match got.types.get(i) {
                Some(&t) => t,
                None => match &got.rest {
                    Some(Rest::Variadic(t)) => *t,
                    Some(Rest::Pack(_)) => Types::ANY,
                    None => Types::NIL,
                },
            };
            let at = values.get(i).map(|v| v.span()).unwrap_or(span);
            // A value left out is `nil`: the first one that cannot be is
            // named, once.
            let missing = got.types.len() <= i && got.rest.is_none();
            if missing && !self.env.assignable(have, want, self.mode) {
                let msg = format!(
                    "missing return value {} of type `{}` (the function returns {} values; one left out is `nil`)",
                    i + 1,
                    self.env.display(want),
                    expected.types.len()
                );
                self.report("E2001", at, msg);
                break;
            }
            self.check_assignable(at, have, want);
        }
        if expected.rest.is_none()
            && got.types.len() > expected.types.len()
            && !expected.types.is_empty()
        {
            let at = values
                .get(expected.types.len())
                .map(|v| v.span())
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

    /// Whether a member annotation of `c` names a `typeof` alias resolved
    /// so far (by its name: another of that name only costs a refresh).
    fn names_typeof_alias(&self, c: ClassDecl<'_>) -> bool {
        if self.env.typeof_alias_names.is_empty() {
            return false;
        }
        let mut names: Vec<String> = Vec::new();
        for m in c.members() {
            match m.kind() {
                ClassMemberKind::Field { ty, .. } => type_ref_names(ty, &mut names),
                ClassMemberKind::Method { func, .. } => {
                    let sig = FnSig::of(func);
                    for p in &sig.params {
                        if let Some(t) = p.ty {
                            type_ref_names(t, &mut names);
                        }
                    }
                    match sig.ret {
                        Some(RetType::Single(t)) => type_ref_names(t, &mut names),
                        Some(RetType::List { types, .. }) => {
                            types.iter().for_each(|t| type_ref_names(t, &mut names))
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        names
            .iter()
            .any(|n| self.env.typeof_alias_names.contains(n.as_str()))
    }

    /// The bodies of a class's methods (spec chapter 3).
    fn class_body(&mut self, c: ClassDecl<'_>) {
        let Some(def) = self.env.lookup_type(c.name().text(), &self.scope) else {
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
        // A field's own `typeof(expr)` is typed where the file reaches the
        // class, as a `local`'s annotation (decision L70).
        let mut own: Vec<Expr<'_>> = Vec::new();
        for m in c.members() {
            if let ClassMemberKind::Field { ty, .. } = m.kind() {
                typeof_exprs(ty, &mut own);
            }
        }
        let has_own = !own.is_empty();
        for x in own {
            let ty = self.infer(x);
            let ty = self.env.types.widen(ty);
            self.env
                .typeof_types
                .insert((x.span().start, x.span().end), ty);
        }
        // Member annotations may use a `typeof` alias the file resolved
        // above (decision L57), or one of another module, resolved when that
        // module was checked (decision L76).
        if has_own
            || self
                .scope
                .module
                .is_some_and(|module| self.env.typeof_resolved.contains(&module))
            || self.names_typeof_alias(c)
        {
            let scope = self.scope.clone();
            self.env.refresh_member_types(def, c, &scope);
        }
        // A base whose type arguments use `typeof` (directly or through a
        // `typeof` alias) is known only where the file reaches the class
        // (`extends Module<typeof(DEFAULTS)>`, decision L45).
        if let Some(e) = c.extends() {
            let mut exprs: Vec<Expr<'_>> = Vec::new();
            typeof_exprs(e, &mut exprs);
            for x in exprs {
                let ty = self.infer(x);
                let ty = self.env.types.widen(ty);
                self.env
                    .typeof_types
                    .insert((x.span().start, x.span().end), ty);
            }
            let scope = self.scope.clone();
            let t = self.env.resolve(e, &scope);
            if let Some(b) = self.env.def_mut(def).bases.first_mut() {
                *b = t;
            }
        }
        if self.mode == Mode::Strict {
            self.definite_assignment(c, def);
        }
        for m in c.members() {
            let is_static = m.modifiers().contains(&Modifier::Static);
            self.classes.push((def, is_static));
            match m.kind() {
                ClassMemberKind::Field {
                    value: Some(v),
                    name,
                    ..
                } => {
                    let want = if is_static {
                        self.env.def(def).statics.get(name.text()).map(|m| m.ty)
                    } else {
                        self.env.def(def).members.get(name.text()).map(|m| m.ty)
                    };
                    // Instance initializers see `self`.
                    self.push_scope();
                    if !is_static {
                        self.declare("self", Some(instance), instance, false, None);
                    }
                    let t = self.check(v, want);
                    if let Some(w) = want {
                        self.check_assignable(v.span(), t, w);
                    }
                    self.pop_scope();
                }
                ClassMemberKind::Method {
                    func,
                    has_body: true,
                    name,
                } => {
                    let expected = if is_static {
                        self.env.def(def).statics.get(name.text()).map(|m| m.ty)
                    } else {
                        self.env.def(def).members.get(name.text()).map(|m| m.ty)
                    };
                    let exp = expected.and_then(|e| self.as_func(e));
                    self.function_body(&FnSig::of(func), exp, (!is_static).then_some(instance));
                }
                ClassMemberKind::Constructor {
                    func,
                    has_body: true,
                } => {
                    let saved = self.ctor_of.replace(def);
                    self.function_body(&FnSig::of(func), None, Some(instance));
                    self.ctor_of = saved;
                }
                ClassMemberKind::Getter { body, ty, .. } => {
                    let fb = FnSig::accessor(body, ty, None);
                    self.function_body(&fb, None, (!is_static).then_some(instance));
                }
                ClassMemberKind::Setter { body, param, .. } => {
                    let fb = FnSig::accessor(body, None, Some(param));
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
    fn definite_assignment(&mut self, c: ClassDecl<'_>, def: DefId) {
        let required: Vec<Name<'_>> = c
            .members()
            .iter()
            .filter(|m| !m.modifiers().contains(&Modifier::Static))
            .filter_map(|m| match m.kind() {
                ClassMemberKind::Field {
                    name,
                    optional: false,
                    value: None,
                    ty,
                } if !matches!(ty.kind(), TypeKind::Optional(_)) => Some(name),
                _ => None,
            })
            .collect();
        if required.is_empty() {
            return;
        }
        let ctor = c.members().iter().find_map(|m| match m.kind() {
            ClassMemberKind::Constructor {
                func,
                has_body: true,
            } => Some(func),
            _ => None,
        });
        let mut walk = DefiniteAssignment::default();
        if let Some(f) = ctor {
            self.definite_block(def, f.body(), &mut walk);
        }
        let DefiniteAssignment {
            assigned,
            stop,
            later,
        } = walk;
        for name in required {
            if !assigned.contains(name.text()) {
                let msg = match &stop {
                    Some(at) if later.contains(name.text()) => format!(
                        "field `{}` is assigned only after {at}; assign it before",
                        name.text()
                    ),
                    _ => format!(
                        "field `{}` is not assigned in the constructor (initialize it, assign it, or make it optional)",
                        name.text()
                    ),
                };
                self.report("E2052", name.span(), msg);
            }
        }
    }

    /// One block of a constructor for [`Self::definite_assignment`]: the
    /// fields it assigns on every path, up to a call of the class's own
    /// method on `self` or a `return` (`stop`); an `if` with an `else`
    /// assigns what every branch does, a `do` block what it does.
    fn definite_block(&mut self, def: DefId, b: Block<'_>, w: &mut DefiniteAssignment) {
        for s in b.stmts() {
            if let StmtKind::Assign { targets, .. } = s.kind() {
                for t in targets {
                    if let ExprKind::Field { obj, name } = t.kind()
                        && matches!(obj.kind(), ExprKind::Name(n) if n.text() == "self")
                    {
                        if w.stop.is_some() {
                            w.later.insert(name.text().into());
                        } else {
                            w.assigned.insert(name.text().into());
                        }
                    }
                }
            }
            if w.stop.is_some() {
                continue;
            }
            match s.kind() {
                StmtKind::Do(inner) => self.definite_block(def, inner, w),
                StmtKind::If {
                    then_block,
                    elseifs,
                    else_block,
                    ..
                } => {
                    let mut blocks = vec![then_block];
                    blocks.extend(elseifs.iter().map(|(_, b)| b));
                    let complete = else_block.is_some();
                    blocks.extend(else_block);
                    // Each branch from what is assigned so far; what all of
                    // them assign is assigned after the `if` when it has an
                    // `else`. A stop in any branch ends the check.
                    let mut all: Option<FxHashSet<Box<str>>> = None;
                    for blk in blocks {
                        let mut bw = DefiniteAssignment {
                            assigned: w.assigned.clone(),
                            stop: None,
                            later: FxHashSet::default(),
                        };
                        self.definite_block(def, blk, &mut bw);
                        if bw.stop.is_some() && w.stop.is_none() {
                            w.stop = bw.stop.clone();
                        }
                        all = Some(match all {
                            None => bw.assigned,
                            Some(a) => a.intersection(&bw.assigned).cloned().collect(),
                        });
                    }
                    if complete && w.stop.is_none() {
                        w.assigned = all.unwrap_or_default();
                    }
                }
                _ => {
                    // A call of a method of wowlua code on `self` (which may
                    // read the fields) or a return ends the check; native
                    // methods (`self:SetSize(…)`) cannot read them.
                    let method = match s.kind() {
                        StmtKind::Call(e) => match e.kind() {
                            ExprKind::Method { obj, name, .. } if matches!(obj.kind(), ExprKind::Name(n) if n.text() == "self") => {
                                Some(name.text())
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    let calls_own = method.is_some_and(|m| {
                        self.env
                            .instance_member(def, &[], m)
                            .is_none_or(|(_, owner)| !self.env.def(owner).native)
                    });
                    if calls_own {
                        w.stop = method.map(|m| format!("`self:{m}()`, which may read it"));
                    } else if matches!(s.kind(), StmtKind::Return(_)) {
                        w.stop = Some("the constructor's `return`".to_string());
                    }
                }
            }
        }
    }

    /// `t` with an open table sealed, and the open tables in its fields.
    fn sealed(&mut self, t: TypeId) -> TypeId {
        self.sealed_at(t, 0)
    }

    fn sealed_at(&mut self, t: TypeId, depth: u32) -> TypeId {
        let Ty::Record(r) = self.env.types.get(t).clone() else {
            return t;
        };
        if depth > 4 {
            return t;
        }
        let mut changed = r.open;
        let mut fields = r.fields.clone();
        let names: Vec<(Box<str>, Field)> = r.fields.iter().map(|(n, f)| (n.into(), *f)).collect();
        for (name, f) in names {
            let nt = self.sealed_at(f.ty, depth + 1);
            if nt != f.ty {
                fields.insert(name, Field { ty: nt, ..f });
                changed = true;
            }
        }
        if !changed {
            return t;
        }
        let mut r = r;
        r.open = false;
        r.fields = fields;
        self.env.types.record(r)
    }

    /// Adds a field to an open table, or checks it against a sealed one.
    /// A field read through `__index` is replaced by the table's own.
    pub fn extend_open(&mut self, owner: TypeId, name: &str, value: TypeId) {
        let Ty::Record(mut r) = self.env.types.get(owner).clone() else {
            return;
        };
        let fallback = self
            .fallback_fields
            .get(&owner)
            .is_some_and(|f| f.contains(name));
        if fallback {
            let mut rest = self.fallback_fields[&owner].clone();
            rest.remove(name);
            r.fields.insert(
                name.into(),
                Field {
                    ty: self.env.types.widen(value),
                    optional: false,
                    readonly: false,
                },
            );
            let new = self.env.types.record(r);
            self.fallback_fields.insert(new, rest);
            self.retype_owner(owner, new);
            return;
        }
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
        if let Some(rest) = self.fallback_fields.get(&owner).cloned() {
            self.fallback_fields.insert(new, rest);
        }
        self.retype_owner(owner, new);
    }

    /// The locals typed `owner` now have the type `new`, and so do the
    /// tables holding it in a field (`API.UI.B = …` extends `API`'s `UI`, so
    /// what `API` is exported as has it too).
    fn retype_owner(&mut self, owner: TypeId, new: TypeId) {
        let vars: Vec<(VarId, TypeId)> = self.flow.iter().map(|(v, t)| (*v, *t)).collect();
        for (v, t) in vars {
            let replaced = if t == owner {
                Some(new)
            } else {
                self.replace_nested(t, owner, new, 0)
            };
            let Some(n) = replaced else { continue };
            self.flow.insert(v, n);
            let (base, assigned) = {
                let var = &self.vars[v.0 as usize];
                (var.base, var.assigned)
            };
            if base == owner || base == t {
                let assigned = if assigned == owner || assigned == t {
                    n
                } else {
                    assigned
                };
                self.set_base(v, n, assigned);
            }
        }
    }

    /// `t` with the record `owner` in its fields replaced by `new`, if it
    /// holds it.
    fn replace_nested(
        &mut self,
        t: TypeId,
        owner: TypeId,
        new: TypeId,
        depth: u32,
    ) -> Option<TypeId> {
        if depth > 4 {
            return None;
        }
        let Ty::Record(r) = self.env.types.get(t).clone() else {
            return None;
        };
        let mut fields = r.fields.clone();
        let mut changed = false;
        let names: Vec<(Box<str>, Field)> = r.fields.iter().map(|(n, f)| (n.into(), *f)).collect();
        for (name, f) in names {
            let nt = if f.ty == owner {
                Some(new)
            } else {
                self.replace_nested(f.ty, owner, new, depth + 1)
            };
            if let Some(nt) = nt {
                fields.insert(name, Field { ty: nt, ..f });
                changed = true;
            }
        }
        if !changed {
            return None;
        }
        let mut r = r;
        r.fields = fields;
        Some(self.env.types.record(r))
    }

    /// The type a class's `self` field access sees (spec §3.1.3).
    pub fn class_of_self(&self) -> Option<DefId> {
        self.classes.last().map(|(d, _)| *d)
    }

    pub fn is_class_def(&self, d: DefId) -> bool {
        self.env.def(d).kind == DefKind::Class
    }
}

/// What a constructor assigns (E2052): the fields set on every path so far,
/// what ended the check, and the fields set only after that.
#[derive(Default)]
struct DefiniteAssignment {
    assigned: FxHashSet<Box<str>>,
    stop: Option<String>,
    later: FxHashSet<Box<str>>,
}

/// Whether the file sets the environment of its own code: `setfenv` with a
/// stack level, or with a function the file defines (by name). Setting the
/// environment of other code (a chunk from `loadstring`) does not change
/// how this file's names resolve.
fn calls_setfenv(b: Block<'_>) -> bool {
    // Names of the functions this file defines.
    let mut defined: FxHashSet<Box<str>> = FxHashSet::default();
    wowlua_syntax::visit::walk_stmts(b, &mut |s| match s.kind() {
        StmtKind::LocalFunction { name, .. } => {
            defined.insert(name.text().into());
        }
        StmtKind::Function { name, .. } => {
            defined.insert(name.path().at(name.path().len() - 1).text().into());
        }
        StmtKind::Local { names, values, .. } => {
            for (n, v) in names.iter().zip(values) {
                if matches!(v.kind(), ExprKind::Function(_)) {
                    defined.insert(n.name.text().into());
                }
            }
        }
        _ => {}
    });
    let mut found = false;
    wowlua_syntax::visit::walk_block(b, &mut |e| {
        if let ExprKind::Call { callee, args } = e.kind()
            && matches!(callee.kind(), ExprKind::Name(n) if n.text() == "setfenv")
        {
            let own = match args.exprs().first().map(|a| a.kind()) {
                Some(ExprKind::Number(_)) | None => true,
                Some(ExprKind::Name(n)) => defined.contains(n.text()),
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
/// A value whose type comes from somewhere other than a literal written
/// here: a local keeps it unwidened (spec §2.4, decision L64).
fn typed_elsewhere(e: Expr<'_>) -> bool {
    match e.kind() {
        ExprKind::Paren(inner) => typed_elsewhere(inner),
        ExprKind::Call { .. }
        | ExprKind::OptCall { .. }
        | ExprKind::Method { .. }
        | ExprKind::OptMethod { .. }
        | ExprKind::Name(_)
        | ExprKind::Field { .. }
        | ExprKind::OptField { .. }
        | ExprKind::Index { .. }
        | ExprKind::OptIndex { .. }
        | ExprKind::Cast { .. } => true,
        _ => false,
    }
}

/// The last name of every type a type annotation refers to (`a.b.C<T>`
/// gives `C` and `T`'s).
pub(crate) fn type_ref_names(t: Type<'_>, out: &mut Vec<String>) {
    match t.kind() {
        TypeKind::Ref { path, args } => {
            if let Some(last) = path.iter().next_back() {
                out.push(last.text().to_string());
            }
            for a in args {
                if let TypeArg::Type(t) = a {
                    type_ref_names(t, out);
                }
            }
        }
        TypeKind::Paren(i) | TypeKind::Optional(i) | TypeKind::Array(i) | TypeKind::KeyOf(i) => {
            type_ref_names(i, out)
        }
        TypeKind::Index { object, key } => {
            type_ref_names(object, out);
            type_ref_names(key, out);
        }
        TypeKind::Union(ms) | TypeKind::Intersection(ms) => {
            ms.iter().for_each(|m| type_ref_names(m, out))
        }
        TypeKind::Table(fields) => {
            for f in fields {
                match f {
                    TableTypeField::Named { ty, .. }
                    | TableTypeField::NamedString { ty, .. }
                    | TableTypeField::Array(ty) => type_ref_names(ty, out),
                    TableTypeField::Indexer { key, value } => {
                        type_ref_names(key, out);
                        type_ref_names(value, out);
                    }
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn typeof_exprs<'t>(t: Type<'t>, out: &mut Vec<Expr<'t>>) {
    match t.kind() {
        TypeKind::Typeof(e) => out.push(e),
        TypeKind::Paren(i) | TypeKind::Optional(i) | TypeKind::Array(i) | TypeKind::KeyOf(i) => {
            typeof_exprs(i, out)
        }
        TypeKind::Index { object, key } => {
            typeof_exprs(object, out);
            typeof_exprs(key, out);
        }
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

fn exported_names(s: Stmt<'_>) -> Vec<String> {
    match s.kind() {
        StmtKind::Local { names, .. } => names.iter().map(|n| n.name.text().to_string()).collect(),
        StmtKind::Function { name, .. } => vec![name.path().at(0).text().to_string()],
        StmtKind::Const { name, .. } => vec![name.text().to_string()],
        StmtKind::Class(c) => vec![c.name().text().to_string()],
        StmtKind::Enum(e) => vec![e.name().text().to_string()],
        _ => Vec::new(),
    }
}

/// The declarations (by name start) of locals that are assigned from a
/// nested function after their declaration or captured and assigned later;
/// and of those assigned from a nested function.
fn reassigned_in_closures(b: Block<'_>) -> (FxHashSet<u32>, FxHashSet<u32>, FxHashSet<u32>) {
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
        fn declare(&mut self, n: Name<'_>) {
            let d = self.depth;
            self.scopes
                .last_mut()
                .expect("scope")
                .insert(n.text().into(), (n.span().start, d));
        }
        fn assign(&mut self, n: Name<'_>) {
            if let Some((start, d)) = self.lookup(n.text()) {
                self.assigned.insert(start);
                if d < self.depth {
                    self.out.insert(start);
                }
            }
        }
        fn read(&mut self, n: Name<'_>) {
            if let Some((start, d)) = self.lookup(n.text())
                && d < self.depth
            {
                self.captured.insert(start);
            }
        }
        fn block(&mut self, b: Block<'_>) {
            self.scopes.push(FxHashMap::default());
            for s in b.stmts() {
                self.stmt(s);
            }
            self.scopes.pop();
        }
        fn func(&mut self, f: FuncNode<'_>) {
            self.depth += 1;
            self.scopes.push(FxHashMap::default());
            for p in f.params() {
                self.declare(p.name());
            }
            self.block(f.body());
            self.scopes.pop();
            self.depth -= 1;
        }
        fn stmt(&mut self, s: Stmt<'_>) {
            match s.kind() {
                StmtKind::Local { names, values } => {
                    for v in values {
                        self.expr(v);
                    }
                    for n in names {
                        self.declare(n.name);
                    }
                }
                StmtKind::LocalFunction { name, func } => {
                    self.declare(name);
                    self.func(func);
                }
                StmtKind::Function { name, func } => {
                    if name.path().len() == 1 && name.method().is_none() {
                        self.assign(name.path().at(0));
                    }
                    self.func(func);
                }
                StmtKind::Assign { targets, values } => {
                    for t in targets {
                        match t.kind() {
                            ExprKind::Name(n) => self.assign(n),
                            _ => self.expr(t),
                        }
                    }
                    for v in values {
                        self.expr(v);
                    }
                }
                StmtKind::Compound { target, value, .. } => {
                    if let ExprKind::Name(n) = target.kind() {
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
                    self.declare(var.name);
                    self.block(body);
                    self.scopes.pop();
                }
                StmtKind::GenericFor { vars, exprs, body } => {
                    for e in exprs {
                        self.expr(e);
                    }
                    self.scopes.push(FxHashMap::default());
                    for v in vars {
                        self.declare(v.name);
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
                    for m in c.members() {
                        match m.kind() {
                            ClassMemberKind::Method { func, .. }
                            | ClassMemberKind::Constructor { func, .. } => self.func(func),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        fn expr(&mut self, e: Expr<'_>) {
            let mut names: Vec<Name<'_>> = Vec::new();
            wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
                if let ExprKind::Name(n) = x.kind() {
                    names.push(n);
                }
            });
            for n in names {
                self.read(n);
            }
            let mut funcs: Vec<FuncNode<'_>> = Vec::new();
            wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
                if let ExprKind::Function(f) = x.kind() {
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

fn literal_string(e: Expr<'_>) -> Option<String> {
    match e.kind() {
        ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value()).into_owned()),
        _ => None,
    }
}

/// The overload set the signatures of `name` among `overloads` make.
fn overload_set_of(
    types: &mut Types,
    overloads: &[(Box<str>, TypeId)],
    name: &str,
) -> Option<TypeId> {
    let sigs: Vec<TypeId> = overloads
        .iter()
        .filter(|(n, _)| n.as_ref() == name)
        .map(|(_, t)| *t)
        .collect();
    match sigs.len() {
        0 => None,
        1 => Some(sigs[0]),
        _ => Some(types.intern(Ty::Intersection(sigs))),
    }
}
