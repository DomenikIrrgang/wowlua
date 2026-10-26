//! Decorators (spec chapter 4): the values of user and wrapping framework
//! decorators, their arguments' names (E1003), whether a decorator accepts
//! its target (E3022), the type it gives the declared name (§4.5.1), and
//! lexical bindings a core decorator's name hides (W3021).

use super::body::Checker;
use super::complete::On;
use super::subst::{self, Subst};
use super::types::*;
use super::types::{Func, Param, Types};
use wowlua_span::Span;
use wowlua_syntax::view::{
    ClassMember, ClassMemberKind, Decorator, Decorators, Expr, ExprKind, Modifier, Name,
};

/// A user (or wrapping framework) decorator of a declaration, evaluated
/// where the declaration is.
#[derive(Clone, Debug)]
pub(crate) struct DecoratorValue {
    pub span: Span,
    /// `@a.b` as written.
    pub name: String,
    /// The decorator's type, `None` where it is not applied to the value
    /// (a registering framework decorator, `@_G.…`).
    pub ty: Option<TypeId>,
}

/// Framework decorators that wrap the function, like a user decorator
/// (the lowerer's `Intrinsic::Wrapping`); the others register it.
fn wrapping(id: &str) -> bool {
    matches!(
        id,
        "internal.wrap"
            | "timer.throttle"
            | "timer.debounce"
            | "timer.nextFrame"
            | "combat.outOfCombat"
    )
}

impl Checker<'_, '_> {
    /// The decorators of a declaration put on `on`, in source order, as
    /// load time evaluates them (spec §4.5.2): each user decorator's
    /// expression (a factory called with its arguments), names in
    /// arguments that are not in scope (E1003), and W3021 for a core
    /// decorator's name that a local function also has.
    pub(crate) fn decorator_values(&mut self, ds: Decorators<'_>, on: On) -> Vec<DecoratorValue> {
        let mut out = Vec::new();
        let user = matches!(
            on,
            On::Function | On::Method | On::Constructor | On::Accessor | On::Field | On::Class
        );
        for d in ds {
            let path = d.path();
            let first = path.at(0).text();
            let name = path.iter().map(|n| n.text()).collect::<Vec<_>>().join(".");
            if d.is_core() {
                self.hidden_by_core(d, first);
                continue;
            }
            // (the lowerer reports a decorator that cannot stand here,
            // E3021, and one that names nothing, E3020)
            if !user || (first != "_G" && self.lookup_var(first).is_none()) {
                continue;
            }
            let id = match path.len() {
                1 => self.intrinsic_path(first, None),
                2 => self.intrinsic_path(first, Some(path.at(1).text())),
                _ => None,
            };
            let registering = id.as_deref().is_some_and(|id| !wrapping(id));
            if first == "_G" || registering {
                // Registering decorators have checks of their own (the
                // event names, the handler's type): their arguments' names
                // only.
                if let Some(args) = d.args() {
                    for e in args.exprs() {
                        self.undefined_in_decorator(e);
                    }
                }
                out.push(DecoratorValue {
                    span: d.span(),
                    name,
                    ty: None,
                });
                continue;
            }
            let outer = self.decorating.replace(self.depth);
            let ty = self.infer(d.expr());
            self.decorating = outer;
            out.push(DecoratorValue {
                span: d.span(),
                name,
                ty: Some(ty),
            });
        }
        out
    }

    /// E1003: a name in a decorator argument that is neither in scope nor
    /// a declared global (the arguments are evaluated where the declaration
    /// is, spec §4.5.2). Function bodies in them run later and are checked
    /// as such.
    fn undefined_in_decorator(&mut self, e: Expr<'_>) {
        let mut names: Vec<Name<'_>> = Vec::new();
        wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
            if let ExprKind::Name(n) = x.kind() {
                names.push(n);
            }
        });
        for n in names {
            if self.lookup_var(n.text()).is_none() && !self.known_global(n.text()) {
                self.report_undefined_in_decorator(n);
            }
        }
    }

    pub(crate) fn report_undefined_in_decorator(&mut self, n: Name<'_>) {
        let msg = format!(
            "`{}` is not a local, an upvalue or a declared global here: decorator arguments are evaluated where the declaration is",
            n.text()
        );
        self.report("E1003", n.span(), msg);
    }

    /// W3021: `@profile` is the core decorator, whatever is in scope; a
    /// local or imported function of that name is ignored there (spec
    /// §4.2). Re-exports of the core decorators (`import profile from
    /// "wowlua:debug"`) are fine.
    fn hidden_by_core(&mut self, d: Decorator<'_>, name: &str) {
        let Some(v) = self.lookup_var(name) else {
            return;
        };
        let reexport = self.import_vars.get(name) == Some(&v) && self.std_imports.contains(name);
        if reexport {
            return;
        }
        let t = self.var_type(v);
        let function = self.env.types.members(t).into_iter().any(|m| {
            matches!(
                self.env.types.get(m),
                Ty::Func(_) | Ty::Function | Ty::Intersection(_)
            )
        });
        if !function {
            return;
        }
        let msg = format!(
            "`@{name}` is the core decorator: the function `{name}` in scope is not used here"
        );
        self.diagnostics.push(
            wowlua_diagnostics::Diagnostic::warning("W3021", msg)
                .with_label(d.span(), "the core decorator")
                .with_help(format!(
                    "to decorate with your function, import it under another name (`import {name} as my{} from …`) and write that name",
                    upper_first(name)
                )),
        );
    }

    /// Applies a declaration's decorators to its value of type `target`,
    /// the one nearest the declaration first (spec §4.5.2): each must
    /// accept the value and the context (E3022). Returns the type the
    /// declared name gets: a decorator that returns a function gives its
    /// result type (spec §4.5.1); `nil` (or `any`) keeps the value.
    pub(crate) fn apply_decorators(
        &mut self,
        values: &[DecoratorValue],
        target: TypeId,
        what: &str,
    ) -> TypeId {
        let ctx = self.decorator_context();
        let mut current = target;
        for v in values.iter().rev() {
            let Some(ty) = v.ty else { continue };
            if let Some(result) = self.decorator_result(v, ty, current, ctx, what) {
                current = result;
            }
        }
        current
    }

    /// [`Self::apply_decorators`]'s type without reporting anything (the
    /// type a function's body sees of itself, before its declaration is
    /// checked and its decorators applied for good).
    pub(crate) fn decorated_quietly(
        &mut self,
        values: &[DecoratorValue],
        target: TypeId,
    ) -> TypeId {
        if values.is_empty() {
            return target;
        }
        let reported = self.diagnostics.len();
        let t = self.apply_decorators(values, target, "function");
        self.diagnostics.truncate(reported);
        t
    }

    /// One decorator applied to a value of type `current`: the type it
    /// gives, if it replaces the value with a function.
    fn decorator_result(
        &mut self,
        v: &DecoratorValue,
        ty: TypeId,
        current: TypeId,
        ctx: TypeId,
        what: &str,
    ) -> Option<TypeId> {
        let ty = self.env.types.non_nil(ty);
        let funcs: Vec<Func> = match self.env.types.get(ty).clone() {
            Ty::Func(f) => vec![Func::clone(&f)],
            Ty::Intersection(parts) => parts.iter().filter_map(|&p| self.as_func(p)).collect(),
            Ty::Any | Ty::Unknown | Ty::Error | Ty::Function | Ty::Never => return None,
            _ => {
                let msg = format!(
                    "`@{}` is `{}`, not a function that can decorate",
                    v.name,
                    self.env.display(ty)
                );
                self.report("E3022", v.span, msg);
                return None;
            }
        };
        let mut first_problem: Option<String> = None;
        for f in &funcs {
            match self.decorator_accepts(f, current, ctx) {
                Ok(result) => {
                    let result = self.env.types.non_nil(result);
                    let function = matches!(self.env.types.get(result), Ty::Func(_))
                        || matches!(self.env.types.get(result), Ty::Intersection(ps)
                            if ps.iter().all(|&p| matches!(self.env.types.get(p), Ty::Func(_))));
                    return function.then_some(result);
                }
                Err(problem) => {
                    first_problem.get_or_insert(problem);
                }
            }
        }
        let problem = first_problem.unwrap_or_default();
        let msg = format!("`@{}` does not accept this {what}: {problem}", v.name);
        self.report("E3022", v.span, msg);
        None
    }

    /// Whether `f` takes `(value, ctx)` (its type parameters bound by
    /// them): its result, or what does not fit.
    fn decorator_accepts(
        &mut self,
        f: &Func,
        value: TypeId,
        ctx: TypeId,
    ) -> Result<TypeId, String> {
        let mut s = Subst::default();
        if let Some(p) = f.params.first() {
            self.unify(p.ty, value, &mut s);
        }
        if let Some(p) = f.params.get(1) {
            self.unify(p.ty, ctx, &mut s);
        }
        let g = subst::apply_func(&mut self.env.types, f, &s);
        let g = self.instantiate_any(&g);
        if let Some(p) = g.params.first()
            && !self.env.assignable(value, p.ty, self.mode)
        {
            return Err(format!(
                "it takes `{}`, and the value is `{}`",
                self.env.display(p.ty),
                self.env.display(value)
            ));
        }
        if let Some(p) = g.params.get(1)
            && !self.env.assignable(ctx, p.ty, self.mode)
        {
            return Err(format!(
                "its second parameter is `{}`, and decorators are called with a `DecoratorContext`",
                self.env.display(p.ty)
            ));
        }
        if let Some(p) = g.params.iter().skip(2).find(|p| !p.optional) {
            return Err(format!(
                "it needs `{}` too, and decorators are called with the value and its context only",
                p.name.as_deref().unwrap_or("a third argument")
            ));
        }
        Ok(g.ret.types.first().copied().unwrap_or(Types::NIL))
    }

    /// A class member's decorators applied to it (E3022): a method's
    /// function takes `self` first; a field has no value (`nil`). A method
    /// gets the type its decorators give (spec §4.5.1).
    pub(crate) fn decorate_member(
        &mut self,
        def: DefId,
        instance: TypeId,
        m: ClassMember<'_>,
        values: &[DecoratorValue],
    ) {
        if values.iter().all(|v| v.ty.is_none()) {
            return;
        }
        let is_static = m.modifiers().contains(&Modifier::Static);
        let member_ty = |c: &Self, name: &str| {
            let d = c.env.def(def);
            if is_static {
                d.statics.get(name)
            } else {
                d.members.get(name)
            }
            .map(|m| m.ty)
        };
        let unit = |c: &mut Self, params: Vec<Param>| {
            c.env.types.func(Func {
                params,
                ret: Pack::empty(),
                ..Func::default()
            })
        };
        let this = |ty: TypeId| Param {
            name: Some("self".into()),
            ty,
            optional: false,
        };
        match m.kind() {
            ClassMemberKind::Method { name, .. } => {
                let Some(declared) = member_ty(self, name.text()) else {
                    return;
                };
                let target = if is_static {
                    declared
                } else {
                    self.env.with_self(declared, instance)
                };
                let decorated = self.apply_decorators(values, target, "method");
                if decorated == target {
                    return;
                }
                let ty = if is_static {
                    decorated
                } else {
                    self.drop_first(decorated)
                };
                self.env
                    .decorated_methods
                    .insert((def, is_static, name.text().into()), declared);
                let d = self.env.def_mut(def);
                let members = if is_static {
                    &mut d.statics
                } else {
                    &mut d.members
                };
                if let Some(member) = members.get_mut(name.text()) {
                    member.ty = ty;
                }
            }
            ClassMemberKind::Constructor { .. } => {
                // `__init(self, …)`: the constructor's parameters.
                let new = self.env.def(def).statics.get("new").map(|m| m.ty);
                let mut params = vec![this(instance)];
                if let Some(f) = new.and_then(|t| self.as_func(t)) {
                    params.extend(f.params);
                }
                let target = unit(self, params);
                self.apply_decorators(values, target, "constructor");
            }
            ClassMemberKind::Getter { name, .. } => {
                let ty = member_ty(self, name.text()).unwrap_or(Types::ANY);
                let target = self.env.types.func(Func {
                    params: vec![this(instance)],
                    ret: Pack::single(ty),
                    ..Func::default()
                });
                self.apply_decorators(values, target, "getter");
            }
            ClassMemberKind::Setter { name, param, .. } => {
                let ty = member_ty(self, name.text()).unwrap_or(Types::ANY);
                let value = Param {
                    name: Some(param.name.text().into()),
                    ty,
                    optional: false,
                };
                let target = unit(self, vec![this(instance), value]);
                self.apply_decorators(values, target, "setter");
            }
            ClassMemberKind::Field { .. } => {
                self.apply_decorators(values, Types::NIL, "field");
            }
        }
    }

    /// A class's decorated methods as declared, for its own body (they get
    /// their decorated types again where the class is checked).
    pub(crate) fn restore_declared_methods(&mut self, def: DefId) {
        let declared: Vec<(bool, Box<str>, TypeId)> = self
            .env
            .decorated_methods
            .iter()
            .filter(|((d, _, _), _)| *d == def)
            .map(|((_, is_static, name), &t)| (*is_static, name.clone(), t))
            .collect();
        for (is_static, name, t) in declared {
            let d = self.env.def_mut(def);
            let members = if is_static {
                &mut d.statics
            } else {
                &mut d.members
            };
            if let Some(m) = members.get_mut(&name) {
                m.ty = t;
            }
        }
    }

    /// The context table's type (`DecoratorContext`, spec §4.5).
    fn decorator_context(&mut self) -> TypeId {
        self.env
            .lookup_type("DecoratorContext", &self.scope)
            .and_then(|d| self.env.def(d).alias)
            .unwrap_or(Types::TABLE)
    }
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}
