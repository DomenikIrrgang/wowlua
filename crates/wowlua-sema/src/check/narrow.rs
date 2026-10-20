//! Flow-sensitive narrowing (spec §2.5): the types of locals along control
//! flow, refined by conditions.

use super::body::{Checker, VarId};
use super::types::Types;
use super::types::*;
use rustc_hash::FxHashMap;
use wowlua_syntax::view::*;

/// The narrowed types of locals at one point of a function.
pub type Flow = FxHashMap<VarId, TypeId>;

/// The root of the field paths of declared globals (`G.a.b`: the path
/// `G.a.b` under this root).
pub const GLOBAL_ROOT: VarId = VarId(u32::MAX);

impl Checker<'_, '_> {
    /// Merges the flows of two paths that meet.
    pub fn join(&mut self, a: &Flow, b: &Flow, a_live: bool, b_live: bool) -> Flow {
        match (a_live, b_live) {
            (true, false) => return a.clone(),
            (false, true) => return b.clone(),
            (false, false) => return a.clone(),
            _ => {}
        }
        let mut out = Flow::default();
        for (v, &ta) in a {
            let tb = b.get(v).copied().unwrap_or_else(|| self.var_base(*v));
            out.insert(*v, self.env.types.union(&[ta, tb]));
        }
        for (v, &tb) in b {
            if !out.contains_key(v) {
                let ta = self.var_base(*v);
                out.insert(*v, self.env.types.union(&[ta, tb]));
            }
        }
        out
    }

    /// `t` where it is truthy: without `nil` and `false`.
    pub fn truthy(&mut self, t: TypeId) -> TypeId {
        let members: Vec<TypeId> = self
            .env
            .types
            .members(t)
            .into_iter()
            .filter_map(|m| match self.env.types.get(m) {
                Ty::Nil | Ty::Bool(false) => None,
                Ty::Boolean => Some(Types::TRUE),
                _ => Some(m),
            })
            .collect();
        self.env.types.union(&members)
    }

    /// `t` where it is falsy: `nil` or `false` (other members cannot be).
    pub fn falsy(&mut self, t: TypeId) -> TypeId {
        let members: Vec<TypeId> = self
            .env
            .types
            .members(t)
            .into_iter()
            .filter_map(|m| match self.env.types.get(m) {
                Ty::Nil | Ty::Bool(false) => Some(m),
                Ty::Boolean => Some(Types::FALSE),
                Ty::Any | Ty::Unknown | Ty::Error => Some(m),
                _ => None,
            })
            .collect();
        self.env.types.union(&members)
    }

    /// The local a condition operand refers to (`x`, `(x)`).
    fn narrowable(&mut self, e: Expr<'_>) -> Option<VarId> {
        match e.kind() {
            ExprKind::Name(n) => self.lookup_var(n.text()),
            ExprKind::Paren(inner) => self.narrowable(inner),
            ExprKind::Field { .. } | ExprKind::OptField { .. } => self.field_var(e, true),
            _ => None,
        }
    }

    /// The root local and dotted path of `a.b.c` (`a` a local).
    pub fn field_path(&self, e: Expr<'_>) -> Option<(VarId, String)> {
        match e.kind() {
            ExprKind::Paren(inner) => self.field_path(inner),
            // `a?.b` names the same field as `a.b`: where it has a value,
            // so does `a.b`.
            ExprKind::Field { obj, name } | ExprKind::OptField { obj, name } => match obj.kind() {
                ExprKind::Name(n) => match self.lookup_var(n.text()) {
                    Some(v) => Some((v, name.text().to_string())),
                    // A declared global: its paths start with its name.
                    None if self.env.declared.has_global(n.text()) => {
                        Some((GLOBAL_ROOT, format!("{}.{}", n.text(), name.text())))
                    }
                    None => None,
                },
                _ => {
                    let (root, path) = self.field_path(obj)?;
                    Some((root, format!("{path}.{}", name.text())))
                }
            },
            _ => None,
        }
    }

    /// The synthetic variable of a field path; `create` makes one (its base
    /// is the field's type as declared).
    pub fn field_var(&mut self, e: Expr<'_>, create: bool) -> Option<VarId> {
        // Nothing to find before a condition narrows a field.
        if !create && self.field_vars.is_empty() {
            return None;
        }
        let (root, path) = self.field_path(e)?;
        if let Some(&v) = self
            .field_vars
            .get(&root)
            .and_then(|paths| paths.get(path.as_str()))
        {
            return Some(v);
        }
        if !create {
            return None;
        }
        let base = self.path_type(e)?;
        if base == Types::ANY || base == Types::ERROR {
            return None;
        }
        let depth = self.vars.get(root.0 as usize).map_or(0, |r| r.depth);
        let v = VarId(self.vars.len() as u32);
        self.vars.push(super::body::Var {
            name: path.as_str().into(),
            declared: Some(base),
            base,
            depth,
            is_const: false,
            reassigned_in_closure: false,
            assigned_in_closure: false,
            never_reassigned: false,
        });
        self.field_vars
            .entry(root)
            .or_default()
            .insert(path.into(), v);
        Some(v)
    }

    /// The type of a field path, without diagnostics: the root's narrowed
    /// type, then each field (narrowed where a condition did).
    fn path_type(&mut self, e: Expr<'_>) -> Option<TypeId> {
        match e.kind() {
            ExprKind::Paren(inner) => self.path_type(inner),
            ExprKind::Name(n) => match self.lookup_var(n.text()) {
                Some(v) => Some(self.var_type(v)),
                None => self.env.declared.global(n.text()),
            },
            ExprKind::Field { obj, name } | ExprKind::OptField { obj, name } => {
                if let Some(v) = self.field_var(e, false)
                    && let Some(&t) = self.flow.get(&v)
                {
                    return Some(t);
                }
                let o = self.path_type(obj)?;
                let o = self.env.types.non_nil(o);
                self.field_type_quiet(o, name.text())
            }
            _ => None,
        }
    }

    /// Drops the narrowing of `root`'s fields: all of them, or `prefix`
    /// and the fields below it.
    pub fn forget_fields(&mut self, root: VarId, prefix: Option<&str>) {
        let Some(paths) = self.field_vars.get(&root) else {
            return;
        };
        let gone: Vec<VarId> = paths
            .iter()
            .filter(|(path, _)| {
                prefix.is_none_or(|p| {
                    path.as_ref() == p
                        || path
                            .strip_prefix(p)
                            .is_some_and(|rest| rest.starts_with('.'))
                })
            })
            .map(|(_, &v)| v)
            .collect();
        for v in gone {
            self.flow.remove(&v);
        }
    }

    /// After a call: locals narrowed to `nil` that closures assign lose
    /// that narrowing (the call may have run such a closure), and so do the
    /// fields of locals passed to it or called as its receiver (it may
    /// change them).
    pub fn forget_passed(&mut self, receiver: Option<Expr<'_>>, args: Args<'_>) {
        // A local narrowed to `nil` that closures assign may hold a value
        // after the call (a callback set it). No local is when the file
        // assigns none in a closure.
        if !self.assigned_in_closures.is_empty() {
            let vars = &self.vars;
            self.flow.retain(|v, t| {
                !(*t == Types::NIL
                    && vars
                        .get(v.0 as usize)
                        .is_some_and(|var| var.assigned_in_closure))
            });
        }
        if self.field_vars.is_empty() {
            return;
        }
        let mut roots = Vec::new();
        let mut note = |c: &Checker<'_, '_>, e: Expr<'_>| {
            let root = match e.kind() {
                ExprKind::Name(n) => c.lookup_var(n.text()).map(|v| (v, None)).or_else(|| {
                    c.env
                        .declared
                        .has_global(n.text())
                        .then(|| (GLOBAL_ROOT, Some(n.text().to_string())))
                }),
                _ => c.field_path(e).map(|(r, p)| {
                    // A global's path: only that global's fields.
                    let global =
                        (r == GLOBAL_ROOT).then(|| p.split('.').next().unwrap_or("").to_string());
                    (r, global)
                }),
            };
            if let Some(r) = root {
                roots.push(r);
            }
        };
        if let Some(r) = receiver {
            note(self, r);
        }
        for a in args.exprs() {
            note(self, a);
        }
        for (r, prefix) in roots {
            self.forget_fields(r, prefix.as_deref());
        }
    }

    /// The flows where `cond` is true and where it is false. The condition
    /// itself must already have been checked.
    pub fn narrow(&mut self, cond: Expr<'_>, flow: &Flow) -> (Flow, Flow) {
        let (mut t_flow, mut f_flow) = self.narrow_cond(cond, flow);
        // `x?.f ~= nil` (or truthy, or equal to a literal): where the chain
        // gives a value, the object of its first optional link exists.
        if let Some((base, when_true)) = chain_value(cond)
            && let Some(v) = self.narrowable(base)
        {
            let target = if when_true { &mut t_flow } else { &mut f_flow };
            let current = self.var_type_in(v, target);
            let present = self.env.types.non_nil(current);
            target.insert(v, present);
        }
        (t_flow, f_flow)
    }

    fn narrow_cond(&mut self, cond: Expr<'_>, flow: &Flow) -> (Flow, Flow) {
        match cond.kind() {
            ExprKind::Paren(inner) => return self.narrow(inner, flow),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => {
                let (t, f) = self.narrow(operand, flow);
                return (f, t);
            }
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow(lhs, flow);
                let (rt, rf) = self.narrow(rhs, &lt);
                let f = self.join(&lf, &rf, true, true);
                return (rt, f);
            }
            ExprKind::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow(lhs, flow);
                let (rt, rf) = self.narrow(rhs, &lf);
                let t = self.join(&lt, &rt, true, true);
                return (t, rf);
            }
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } => {
                let (eq_t, eq_f) = self.narrow_eq(lhs, rhs, flow);
                return if op == BinOp::Eq {
                    (eq_t, eq_f)
                } else {
                    (eq_f, eq_t)
                };
            }
            ExprKind::Call { callee, args } => {
                let (mut t_flow, mut f_flow) = (flow.clone(), flow.clone());
                self.narrow_predicate(callee, args, flow, &mut t_flow, &mut f_flow);
                return (t_flow, f_flow);
            }
            _ => {}
        }
        let (mut t_flow, mut f_flow) = (flow.clone(), flow.clone());
        if let Some(v) = self.narrowable(cond) {
            let current = self.var_type_in(v, flow);
            let t = self.truthy(current);
            let f = self.falsy(current);
            t_flow.insert(v, t);
            f_flow.insert(v, f);
        }
        (t_flow, f_flow)
    }

    /// `x == nil`, `type(x) == "string"`, `x == "literal"`.
    fn narrow_eq(&mut self, lhs: Expr<'_>, rhs: Expr<'_>, flow: &Flow) -> (Flow, Flow) {
        let mut t_flow = flow.clone();
        let mut f_flow = flow.clone();
        let field_of_local = |e: Expr<'_>, c: &Self| matches!(e.kind(), ExprKind::Field { obj, .. } if c.field_path(e).is_some() || matches!(obj.kind(), ExprKind::Name(n) if c.lookup_var(n.text()).is_some()));
        let (subject, other) = if field_of_local(lhs, self)
            || self.narrowable(lhs).is_some()
            || type_call(lhs).is_some()
        {
            (lhs, rhs)
        } else {
            (rhs, lhs)
        };
        // type(x) == "name"
        if let Some(arg) = type_call(subject)
            && let Some(v) = self.narrowable(arg)
            && let ExprKind::String(s) = other.kind()
        {
            let name = String::from_utf8_lossy(&s.value()).into_owned();
            let current = self.var_type_in(v, flow);
            let (yes, no) = self.split_by_type_name(current, &name);
            t_flow.insert(v, yes);
            f_flow.insert(v, no);
            return (t_flow, f_flow);
        }
        let literal = self.literal_of(other);
        // `obj.kind == "a"`: a discriminated union `obj` narrows to the
        // members whose `kind` can be "a" (spec §2.5).
        if let ExprKind::Field { obj, name } = subject.kind()
            && let Some(lit) = literal
            && let Some(owner) = self.narrowable(obj)
        {
            let current = self.var_type_in(owner, flow);
            let members = self.env.types.members(current);
            if members.len() > 1 {
                let mut yes = Vec::new();
                let mut no = Vec::new();
                for m in members {
                    let field = match self.env.types.get(m) {
                        Ty::Record(_) | Ty::Named(..) => self.field_type_quiet(m, name.text()),
                        _ => None,
                    };
                    match field {
                        Some(ft) => {
                            let can = self.env.types.members(ft).into_iter().any(|f| {
                                f == lit
                                    || matches!(self.env.types.get(f), Ty::Any | Ty::Unknown)
                                    || self.literal_fits(lit, f)
                            });
                            if can {
                                yes.push(m);
                            }
                            if ft != lit {
                                no.push(m);
                            }
                        }
                        None => {
                            yes.push(m);
                            no.push(m);
                        }
                    }
                }
                let yes = self.env.types.union(&yes);
                let no = self.env.types.union(&no);
                t_flow.insert(owner, yes);
                f_flow.insert(owner, no);
            }
        }
        let Some(v) = self.narrowable(subject) else {
            return (t_flow, f_flow);
        };
        let current = self.var_type_in(v, flow);
        let literal = match other.kind() {
            ExprKind::Nil => Some(Types::NIL),
            ExprKind::True => Some(Types::TRUE),
            ExprKind::False => Some(Types::FALSE),
            ExprKind::String(s) => Some(self.env.types.str(&String::from_utf8_lossy(&s.value()))),
            ExprKind::Number(n) => n.value().map(|x| self.env.types.num(x)),
            _ => None,
        };
        if let Some(lit) = literal {
            let members = self.env.types.members(current);
            // Equal: the members that can hold the literal.
            let yes: Vec<TypeId> = members
                .iter()
                .copied()
                .filter(|&m| {
                    m == lit
                        || matches!(self.env.types.get(m), Ty::Any | Ty::Unknown | Ty::Error)
                        || self.literal_fits(lit, m)
                })
                .map(|m| {
                    if matches!(self.env.types.get(m), Ty::Any | Ty::Unknown | Ty::Error) {
                        m
                    } else {
                        lit
                    }
                })
                .collect();
            // Not equal: drop the member only if it is exactly the literal.
            let no: Vec<TypeId> = members.iter().copied().filter(|&m| m != lit).collect();
            let yes = self.env.types.union(&yes);
            let no = self.env.types.union(&no);
            t_flow.insert(v, yes);
            f_flow.insert(v, no);
        }
        (t_flow, f_flow)
    }

    /// The literal type of `nil`, `true`, `false`, a string or a number.
    fn literal_of(&mut self, e: Expr<'_>) -> Option<TypeId> {
        match e.kind() {
            ExprKind::Nil => Some(Types::NIL),
            ExprKind::True => Some(Types::TRUE),
            ExprKind::False => Some(Types::FALSE),
            ExprKind::String(s) => Some(self.env.types.str(&String::from_utf8_lossy(&s.value()))),
            ExprKind::Number(n) => n.value().map(|x| self.env.types.num(x)),
            _ => None,
        }
    }

    fn literal_fits(&self, lit: TypeId, member: TypeId) -> bool {
        matches!(
            (self.env.types.get(lit), self.env.types.get(member)),
            (Ty::Str(_), Ty::String) | (Ty::Num(_), Ty::Number) | (Ty::Bool(_), Ty::Boolean)
        )
    }

    /// The members of `t` with Lua type `name`, and the others.
    pub fn split_by_type_name(&mut self, t: TypeId, name: &str) -> (TypeId, TypeId) {
        let target = match name {
            "nil" => Types::NIL,
            "boolean" => Types::BOOLEAN,
            "number" => Types::NUMBER,
            "string" => Types::STRING,
            "table" => Types::TABLE,
            "function" => Types::FUNCTION,
            "thread" => Types::THREAD,
            "userdata" => Types::USERDATA,
            _ => return (t, t),
        };
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for m in self.env.types.members(t) {
            match self.lua_type_of(m) {
                Some(n) if n == name => yes.push(m),
                Some(_) => no.push(m),
                None => {
                    // `any`, `unknown`: narrowed to the named type.
                    yes.push(target);
                    no.push(m);
                }
            }
        }
        (self.env.types.union(&yes), self.env.types.union(&no))
    }

    /// The `type()` name of values of `t`, if fixed.
    fn lua_type_of(&self, t: TypeId) -> Option<&'static str> {
        Some(match self.env.types.get(t) {
            Ty::Nil => "nil",
            Ty::Boolean | Ty::Bool(_) => "boolean",
            Ty::Number | Ty::Num(_) => "number",
            Ty::String | Ty::Str(_) => "string",
            Ty::Table
            | Ty::Array(_)
            | Ty::Map(..)
            | Ty::Record(_)
            | Ty::ClassOf(_)
            | Ty::Module(_) => "table",
            Ty::Named(d, _) => match self.env.def(*d).kind {
                super::env::DefKind::Enum => return None,
                _ if self.env.def(*d).widget.is_some() => "table",
                _ => "table",
            },
            Ty::EnumMember(..) => return None,
            Ty::Function | Ty::Func(_) | Ty::Intersection(_) => "function",
            Ty::Thread => "thread",
            Ty::Userdata => "userdata",
            _ => return None,
        })
    }

    /// A call of a type predicate (`isButton(x)`, spec §2.2.8).
    fn narrow_predicate(
        &mut self,
        callee: Expr<'_>,
        args: Args<'_>,
        flow: &Flow,
        t_flow: &mut Flow,
        f_flow: &mut Flow,
    ) {
        if let ExprKind::Name(n) = callee.kind()
            && n.text() == "canaccessvalue"
            && self.lookup_var("canaccessvalue").is_none()
        {
            self.narrow_accessible(args, flow, t_flow, f_flow);
            return;
        }
        let callee_ty = self.peek_type(callee);
        let Ty::Func(f) = self.env.types.get(callee_ty).clone() else {
            return;
        };
        let Some((index, target)) = f.predicate else {
            return;
        };
        let args = args.exprs();
        let Some(arg) = args.get(index) else {
            return;
        };
        let Some(v) = self.narrowable(arg) else {
            return;
        };
        let current = self.var_type_in(v, flow);
        // A generic predicate (`v: V | Secret<V>`, `v is V`): its type
        // parameters are inferred from the argument's narrowed type, and
        // the ones it does not bind are `any`.
        let target = if f.type_params.is_empty() {
            target
        } else {
            let mut s = super::subst::Subst::default();
            if let Some(p) = f.params.get(index) {
                self.unify(p.ty, current, &mut s);
            }
            for p in &f.type_params {
                if self.env.params[p.0 as usize].pack {
                    s.packs.entry(*p).or_insert_with(Pack::any);
                } else {
                    s.types.entry(*p).or_insert(Types::ANY);
                }
            }
            super::subst::apply(&mut self.env.types, target, &s)
        };
        // Each member is `T` (true branch), not `T` (false branch), or may
        // be either: an `any`, or a supertype of `T` (narrowed to `T`). A
        // branch no member reaches is `never`: unreachable.
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for m in self.env.types.members(current) {
            let open = match self.env.types.get(m) {
                Ty::Any | Ty::Error => Some(m),
                Ty::Unknown => Some(target),
                _ => None,
            };
            match open {
                Some(narrowed) => {
                    yes.push(narrowed);
                    no.push(m);
                }
                None if self.env.assignable(m, target, super::query::Mode::Strict) => yes.push(m),
                None if self.env.assignable(target, m, super::query::Mode::Strict) => {
                    yes.push(target);
                    no.push(m);
                }
                None => no.push(m),
            }
        }
        let yes = self.env.types.union(&yes);
        // `v is any` excludes nothing.
        let no = match self.env.types.get(target) {
            Ty::Any | Ty::Unknown | Ty::Error => current,
            _ => self.env.types.union(&no),
        };
        t_flow.insert(v, yes);
        f_flow.insert(v, no);
    }

    /// Does a type predicate (or `canaccessvalue`) in `cond` leave one of
    /// its arguments `never` in `flow`, the flow where `cond` is true or
    /// false: is that branch unreachable (spec §2.2.8)?
    pub fn excluded_by_predicate(&mut self, cond: Expr<'_>, flow: &Flow) -> bool {
        let mut args = Vec::new();
        wowlua_syntax::visit::walk_expr_shallow(cond, &mut |e| {
            let ExprKind::Call { callee, args: a } = e.kind() else {
                return;
            };
            args.push((callee, a));
        });
        for (callee, a) in args {
            let index = if matches!(callee.kind(), ExprKind::Name(n) if n.text() == "canaccessvalue")
                && self.lookup_var("canaccessvalue").is_none()
            {
                0
            } else {
                let callee_ty = self.peek_type(callee);
                match self.env.types.get(callee_ty) {
                    Ty::Func(f) => match f.predicate {
                        Some((index, _)) => index,
                        None => continue,
                    },
                    _ => continue,
                }
            };
            let Some(arg) = a.exprs().get(index) else {
                continue;
            };
            let v = match arg.kind() {
                ExprKind::Name(n) => self.lookup_var(n.text()),
                ExprKind::Field { .. } | ExprKind::OptField { .. } => self.field_var(arg, false),
                _ => None,
            };
            if v.is_some_and(|v| flow.get(&v) == Some(&Types::NEVER)) {
                return true;
            }
        }
        false
    }

    /// `canaccessvalue(x)`, the complement of the predicate `issecretvalue`
    /// (spec §2.11): where it is true, `x` has no `Secret<…>` members; where
    /// it is false, `x` is one of them (a value that is not secret is always
    /// accessible).
    fn narrow_accessible(
        &mut self,
        args: Args<'_>,
        flow: &Flow,
        t_flow: &mut Flow,
        f_flow: &mut Flow,
    ) {
        let Some(arg) = args.exprs().first() else {
            return;
        };
        let Some(v) = self.narrowable(arg) else {
            return;
        };
        let current = self.var_type_in(v, flow);
        let mut plain = Vec::new();
        let mut secret = Vec::new();
        for m in self.env.types.members(current) {
            match self.env.types.get(m) {
                Ty::Secret(_) => secret.push(m),
                Ty::Any | Ty::Unknown | Ty::Error => {
                    plain.push(m);
                    secret.push(m);
                }
                _ => plain.push(m),
            }
        }
        // A branch no member reaches is `never`: unreachable.
        let plain = self.env.types.union(&plain);
        let secret = self.env.types.union(&secret);
        t_flow.insert(v, plain);
        f_flow.insert(v, secret);
    }
}

/// `type(x)`: its argument.
/// The object of an optional chain's first optional link (`x` in
/// `x?.f.g`, `x.y` in `x.y?:m()`).
fn optional_base(e: Expr<'_>) -> Option<Expr<'_>> {
    match e.kind() {
        ExprKind::OptField { obj, .. }
        | ExprKind::OptIndex { obj, .. }
        | ExprKind::OptMethod { obj, .. } => optional_base(obj).or(Some(obj)),
        ExprKind::OptCall { callee, .. } => optional_base(callee).or(Some(callee)),
        ExprKind::Field { obj, .. }
        | ExprKind::Index { obj, .. }
        | ExprKind::Method { obj, .. } => optional_base(obj),
        ExprKind::Call { callee, .. } => optional_base(callee),
        _ => None,
    }
}

/// A condition that holds only when an optional chain gives a value: the
/// chain's base, and whether that is the true branch.
fn chain_value(cond: Expr<'_>) -> Option<(Expr<'_>, bool)> {
    match cond.kind() {
        ExprKind::Paren(inner) => chain_value(inner),
        ExprKind::Binary {
            op: op @ (BinOp::Eq | BinOp::Ne),
            lhs,
            rhs,
        } => {
            let (chain, other) = if optional_base(lhs).is_some() {
                (lhs, rhs)
            } else {
                (rhs, lhs)
            };
            let base = optional_base(chain)?;
            match (other.kind(), op) {
                (ExprKind::Nil, BinOp::Ne) => Some((base, true)),
                (ExprKind::Nil, BinOp::Eq) => Some((base, false)),
                (
                    ExprKind::String(_) | ExprKind::Number(_) | ExprKind::True | ExprKind::False,
                    BinOp::Eq,
                ) => Some((base, true)),
                _ => None,
            }
        }
        _ => optional_base(cond).map(|b| (b, true)),
    }
}

fn type_call(e: Expr<'_>) -> Option<Expr<'_>> {
    let ExprKind::Call { callee, args } = e.kind() else {
        return None;
    };
    let ExprKind::Name(n) = callee.kind() else {
        return None;
    };
    if n.text() != "type" {
        return None;
    }
    args.exprs().exactly().map(|[only]| only)
}
