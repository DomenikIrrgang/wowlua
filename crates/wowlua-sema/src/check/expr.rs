//! Expressions (spec chapter 2): inference and contextual checking, names
//! and globals, tables, fields, operators and calls (arity, generics,
//! overloads).

use super::body::Checker;
use super::env::{DefKind, MemberKind, Visibility};
use super::subst::{self, Subst};
use super::types::Field;
use super::types::*;
use wowlua_span::Span;
use wowlua_syntax::ast::*;

impl Checker<'_> {
    /// The type of an expression.
    pub fn infer(&mut self, e: &Expr) -> TypeId {
        self.check(e, None)
    }

    /// The type of an expression where `expected` is wanted (contextual
    /// typing, spec §2.4.3). Mismatches are reported by the caller.
    pub fn check(&mut self, e: &Expr, expected: Option<TypeId>) -> TypeId {
        let pack = stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_inner(e, expected));
        let t = pack
            .types
            .first()
            .copied()
            .unwrap_or_else(|| match &pack.rest {
                Some(Rest::Variadic(t)) => self.env.types.optional(*t),
                _ => Types::NIL,
            });
        self.expr_types.insert((e.span.start, e.span.end), t);
        t
    }

    /// All values of a multi-value expression (calls, `...`).
    pub fn infer_multi(&mut self, e: &Expr) -> Pack {
        let pack = stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_inner(e, None));
        let first = pack.types.first().copied().unwrap_or(Types::NIL);
        self.expr_types.insert((e.span.start, e.span.end), first);
        pack
    }

    /// The type an expression was checked with (for predicates).
    pub fn peek_type(&self, e: &Expr) -> TypeId {
        self.expr_types
            .get(&(e.span.start, e.span.end))
            .copied()
            .unwrap_or(Types::ANY)
    }

    /// A list of expressions as values (the last one expands).
    pub fn expr_list(&mut self, es: &[Expr], expected: &[Option<TypeId>]) -> Vec<TypeId> {
        let pack = self.expr_pack(es, expected);
        let mut out = pack.types.clone();
        if let Some(Rest::Variadic(t)) = pack.rest {
            while out.len() < expected.len().max(es.len()) {
                let o = self.env.types.optional(t);
                out.push(o);
            }
        }
        out
    }

    pub fn expr_pack(&mut self, es: &[Expr], expected: &[Option<TypeId>]) -> Pack {
        let mut types = Vec::new();
        let mut rest = None;
        for (i, e) in es.iter().enumerate() {
            let want = expected.get(i).copied().flatten();
            if i + 1 == es.len() && is_multi(e) {
                let mut p = self.expr_inner(e, want);
                if let (Some(w), Some(&first)) = (want, p.types.first()) {
                    let _ = (w, first);
                }
                let first = p.types.first().copied().unwrap_or(Types::NIL);
                self.expr_types.insert((e.span.start, e.span.end), first);
                types.append(&mut p.types);
                rest = p.rest;
            } else {
                types.push(self.check(e, want));
            }
        }
        Pack { types, rest }
    }

    fn expr_inner(&mut self, e: &Expr, expected: Option<TypeId>) -> Pack {
        let single = |t: TypeId| Pack::single(t);
        match &e.kind {
            ExprKind::Nil => single(Types::NIL),
            ExprKind::True => single(Types::TRUE),
            ExprKind::False => single(Types::FALSE),
            ExprKind::Number(n) => single(match n.value {
                Some(v) => self.env.types.num(v),
                None => Types::NUMBER,
            }),
            ExprKind::String(s) => single(self.env.types.str(&String::from_utf8_lossy(&s.value))),
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.infer(x);
                    }
                }
                single(Types::STRING)
            }
            ExprKind::Vararg => self.vararg_pack(),
            ExprKind::Function(f) => {
                let exp = expected.and_then(|t| self.contextual_func(t));
                // A function passed as an argument or with an expected type
                // is a callback.
                self.callback_next = expected.is_some() || self.callbacks.contains(&e.span);
                single(self.function_body(f, exp, None))
            }
            ExprKind::Table(fields) => single(self.table(fields, expected)),
            // An existence test of an undeclared global: `nil`; inside the
            // region the test guards: `any`.
            ExprKind::Name(n)
                if self.lookup_var(&n.text).is_none()
                    && !self.env.globals.contains_key(n.text.as_ref())
                    && !self.dynamic_env
                    && n.text.as_ref() != "_G"
                    && !self.extra_globals.contains(n.text.as_ref())
                    && !self.assigned_globals.contains(n.text.as_ref())
                    && (self.probes.contains(&e.span)
                        || self.guard.present.contains(n.text.as_ref())) =>
            {
                single(if self.probes.contains(&e.span) {
                    Types::NIL
                } else {
                    Types::ANY
                })
            }
            ExprKind::Name(n) if self.probes.contains(&e.span) => {
                let saved = self.shared.take();
                let t = self.name_type(n);
                self.shared = saved;
                single(t)
            }
            ExprKind::Name(n) => single(self.name_type(n)),
            ExprKind::Paren(inner) => single(self.check(inner, expected)),
            ExprKind::Cast { expr, ty } => {
                let from = self.infer(expr);
                let to = self.annotation(ty);
                if let Some(c) = self.value_check(to) {
                    self.value_checks.insert(e.span, c);
                }
                // Assertions between incomparable types (E2015).
                if !self.env.assignable(from, to, self.mode)
                    && !self.env.assignable(to, from, self.mode)
                    && !self.comparable(from, to)
                {
                    let msg = format!(
                        "cannot assert `{}` as `{}`",
                        self.env.display(from),
                        self.env.display(to)
                    );
                    self.report("E2015", e.span, msg);
                }
                single(to)
            }
            ExprKind::Field { obj, name } | ExprKind::OptField { obj, name } => {
                if self.is_super(obj) {
                    return single(self.super_member(&name.text, name.span));
                }
                let probed = self.probes.contains(&e.span);
                let path = self.api_path_of(e);
                if self.shared.is_some()
                    && !probed
                    && let Some(path) = &path
                {
                    self.check_available(path, name.span);
                }
                let guarded = path
                    .as_ref()
                    .is_some_and(|p| self.guard.present.contains(p));
                let o = self.infer(obj);
                let o = if matches!(e.kind, ExprKind::OptField { .. }) {
                    self.env.types.non_nil(o)
                } else {
                    o
                };
                // A member the type lacks, in an existence test (`nil`) or
                // in the region such a test guards (`any`).
                let missing = (probed || guarded)
                    && self.member_type(o, &name.text, name.span, true) == Some(Types::ERROR);
                let t = if missing {
                    if probed { Types::NIL } else { Types::ANY }
                } else {
                    self.member_type(o, &name.text, name.span, false)
                        .unwrap_or(Types::ANY)
                };
                let t = if matches!(e.kind, ExprKind::OptField { .. }) {
                    self.env.types.optional(t)
                } else {
                    t
                };
                // A field a condition narrowed.
                if let Some(v) = self.field_var(e, false)
                    && let Some(&n) = self.flow.get(&v)
                {
                    return single(n);
                }
                single(t)
            }
            ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
                let o = self.infer(obj);
                let k = self.infer(key);
                single(self.index_type(o, k, key, e.span))
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                self.mark_callbacks(args);
                if let ExprKind::Name(n) = &callee.kind
                    && n.text.as_ref() == "super"
                    && !self.classes.is_empty()
                {
                    self.args_types(args, &[]);
                    return Pack::empty();
                }
                let c = self.infer(callee);
                let c = if matches!(e.kind, ExprKind::OptCall { .. }) {
                    self.env.types.non_nil(c)
                } else {
                    c
                };
                // A method called with `.` and no receiver (E2008).
                if args.exprs().is_empty()
                    && let ExprKind::Field { obj, name } = &callee.kind
                    && self.is_method_member(obj, &name.text)
                {
                    self.report(
                        "E2008",
                        e.span,
                        format!("`{}` is a method; call it with `:`", name.text),
                    );
                    return Pack::any();
                }
                self.check_yield_boundary(callee, args);
                let c = self.stdlib_call_type(callee, args, c);
                let mut results = self.call(c, None, args, e.span, expected);
                self.forget_passed(None, args);
                self.stdlib_after_call(callee, args);
                // `assert(v)` returns `v` only where it is truthy.
                if let ExprKind::Name(n) = &callee.kind
                    && n.text.as_ref() == "assert"
                    && self.lookup_var("assert").is_none()
                    && let Some(first) = results.types.first_mut()
                {
                    *first = self.truthy(*first);
                }
                if let Some(t) = self.saved_bind_type(callee, args) {
                    return Pack::single(t);
                }
                if self.secret_locale_call(callee, args) {
                    let secret = self.env.types.intern(Ty::Secret(Types::STRING));
                    return Pack::single(self.env.types.union(&[Types::STRING, secret]));
                }
                // `setmetatable(t, mt)`: `t` with `mt.__index` as fallback.
                if let ExprKind::Name(n) = &callee.kind
                    && n.text.as_ref() == "setmetatable"
                    && self.lookup_var("setmetatable").is_none()
                    && let [t, mt] = args.exprs().as_slice()
                {
                    return Pack::single(self.with_metatable(t, mt));
                }
                results
            }
            ExprKind::Method { obj, name, args } | ExprKind::OptMethod { obj, name, args } => {
                self.mark_callbacks(args);
                if self.is_super(obj) {
                    let m = self.super_member(&name.text, name.span);
                    let instance = self.scope.self_ty.unwrap_or(Types::ANY);
                    return self.call(m, Some(instance), args, e.span, expected);
                }
                let o = self.infer(obj);
                let o = if matches!(e.kind, ExprKind::OptMethod { .. }) {
                    self.env.types.non_nil(o)
                } else {
                    o
                };
                // A function field a condition narrowed (`if t.f then t:f() end`).
                let narrowed = self
                    .field_path(obj)
                    .map(|(r, p)| (r, format!("{p}.{}", name.text)))
                    .or_else(|| match &obj.kind {
                        ExprKind::Name(n) => match self.lookup_var(&n.text) {
                            Some(r) => Some((r, name.text.to_string())),
                            None => self.env.globals.contains_key(n.text.as_ref()).then(|| {
                                (
                                    super::narrow::GLOBAL_ROOT,
                                    format!("{}.{}", n.text, name.text),
                                )
                            }),
                        },
                        _ => None,
                    })
                    .and_then(|key| {
                        self.field_vars
                            .get(&(key.0, key.1.as_str().into()))
                            .copied()
                    })
                    .and_then(|v| self.flow.get(&v).copied());
                let m = match narrowed {
                    Some(t) => self.drop_first(t),
                    None => self.method_type(o, &name.text, name.span),
                };
                let m = self.registry_method_type(o, &name.text, args, m);
                let results = self.call(m, Some(o), args, e.span, expected);
                self.forget_passed(Some(obj), args);
                results
            }
            ExprKind::Binary { op, lhs, rhs } => {
                single(self.binary(*op, lhs, rhs, e.span, expected))
            }
            ExprKind::Unary { op, operand } => {
                if *op == UnOp::Not {
                    self.mark_probes(operand);
                }
                let t = self.infer(operand);
                single(match op {
                    UnOp::Not => Types::BOOLEAN,
                    UnOp::Len => {
                        self.check_secret(t, operand.span);
                        if self.mode == super::query::Mode::Strict
                            && t != Types::ANY
                            && self.env.types.is_optional(t)
                        {
                            let msg = self.nil_message(t);
                            self.report("E2005", operand.span, msg);
                        }
                        Types::NUMBER
                    }
                    UnOp::Neg | UnOp::BNot => {
                        self.check_secret(t, operand.span);
                        self.arith_operand(t, operand.span);
                        Types::NUMBER
                    }
                })
            }
            ExprKind::Error => single(Types::ERROR),
        }
    }

    fn comparable(&self, a: TypeId, b: TypeId) -> bool {
        // Tables are comparable with tables (casts between shapes).
        let tableish = |t: &Ty| {
            matches!(
                t,
                Ty::Table
                    | Ty::Record(_)
                    | Ty::Array(_)
                    | Ty::Map(..)
                    | Ty::Named(..)
                    | Ty::ClassOf(_)
            )
        };
        tableish(self.env.types.get(a)) && tableish(self.env.types.get(b))
    }

    fn vararg_pack(&mut self) -> Pack {
        if self.depth == 0 {
            if let Some((name, ns)) = self.top_vararg {
                return Pack {
                    types: vec![name, ns],
                    rest: None,
                };
            }
            return Pack::any();
        }
        match self.current_vararg() {
            Some(t) => Pack {
                types: Vec::new(),
                rest: Some(Rest::Variadic(t)),
            },
            None => Pack::any(),
        }
    }

    /// The function type an expected type offers to a function expression.
    fn contextual_func(&mut self, t: TypeId) -> Option<Func> {
        for m in self.env.types.members(t) {
            match self.env.types.get(m).clone() {
                Ty::Func(f) => return Some(f),
                Ty::Intersection(ms) => {
                    for x in ms {
                        if let Ty::Func(f) = self.env.types.get(x).clone() {
                            return Some(f);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    // ------------------------------------------------------------ names --

    /// Records the function expressions among call arguments (callbacks).
    fn mark_callbacks(&mut self, args: &Args) {
        for a in args.exprs() {
            let mut a = a;
            while let ExprKind::Paren(inner) = &a.kind {
                a = inner;
            }
            if matches!(a.kind, ExprKind::Function(_)) {
                self.callbacks.insert(a.span);
            }
        }
    }

    /// The type of a name: a local, an import, or a global.
    pub fn name_type(&mut self, n: &Name) -> TypeId {
        if let Some(v) = self.lookup_var(&n.text) {
            return self.var_type(v);
        }
        self.check_available(&n.text, n.span);
        if n.text.as_ref() == "super" && !self.classes.is_empty() {
            return self.super_member("", n.span);
        }
        match self.env.globals.get(n.text.as_ref()) {
            Some(&t) => t,
            None if n.text.as_ref() == "_G" => Types::TABLE,
            None if self.dynamic_env
                || self.extra_globals.contains(n.text.as_ref())
                || self.assigned_globals.contains(n.text.as_ref()) =>
            {
                Types::ANY
            }
            None => {
                let (code, msg) = match self.mode {
                    super::query::Mode::Strict => {
                        ("E1001", format!("`{}` is not a declared global", n.text))
                    }
                    super::query::Mode::Basic => {
                        ("W1001", format!("`{}` is not a declared global", n.text))
                    }
                };
                self.report(code, n.span, msg);
                Types::ANY
            }
        }
    }

    /// Assigning a global (W1002 / E1002 unless declared).
    pub fn assign_global(&mut self, n: &Name, value: TypeId) {
        self.forget_fields(super::narrow::GLOBAL_ROOT, Some(&n.text));
        match self.env.globals.get(n.text.as_ref()).copied() {
            Some(t) => self.check_assignable(n.span, value, t),
            None if self.dynamic_env || self.extra_globals.contains(n.text.as_ref()) => {}
            None => {
                let code = match self.mode {
                    super::query::Mode::Strict => "E1002",
                    super::query::Mode::Basic => "W1002",
                };
                self.report(
                    code,
                    n.span,
                    format!("assigns the undeclared global `{}` (declare it, use `_G.{}`, or make it `local`)", n.text, n.text),
                );
            }
        }
    }

    /// The type an assignment target expects (for contextual typing).
    pub fn target_type(&mut self, t: &Expr) -> Option<TypeId> {
        match &t.kind {
            ExprKind::Name(n) => match self.lookup_var(&n.text) {
                Some(v) => self.vars[v.0 as usize].declared,
                None => self.env.globals.get(n.text.as_ref()).copied(),
            },
            ExprKind::Field { obj, name } => {
                let o = self.peek_or_infer(obj);
                self.field_type_quiet(o, &name.text)
            }
            _ => None,
        }
    }

    fn peek_or_infer(&mut self, e: &Expr) -> TypeId {
        match &e.kind {
            ExprKind::Name(n) => match self.lookup_var(&n.text) {
                Some(v) => self.var_type(v),
                None => self
                    .env
                    .globals
                    .get(n.text.as_ref())
                    .copied()
                    .unwrap_or(Types::ANY),
            },
            _ => self.infer(e),
        }
    }

    /// A field's declared type, without diagnostics.
    /// E2005's message: the value is `nil`, or may be.
    fn nil_message(&self, t: TypeId) -> String {
        if t == Types::NIL {
            "the value is always `nil` here".into()
        } else {
            format!("`{}` may be `nil`", self.env.display(t))
        }
    }

    pub(crate) fn field_type_quiet(&mut self, o: TypeId, name: &str) -> Option<TypeId> {
        match self.env.types.get(o).clone() {
            // Every member's field.
            Ty::Union(members) => {
                let mut out = Vec::new();
                for m in members {
                    out.push(self.field_type_quiet(m, name)?);
                }
                Some(self.env.types.union(&out))
            }
            Ty::Record(r) => r.fields.get(name).map(|f| f.ty),
            // A method's stored value takes `self` first.
            Ty::Named(d, args) => self.env.instance_member(d, &args, name).map(|(m, _)| {
                if m.kind == MemberKind::Method {
                    self.env.with_self(m.ty, o)
                } else {
                    m.ty
                }
            }),
            Ty::ClassOf(d) => self
                .env
                .static_member(d, name)
                .map(|m| m.ty)
                .or_else(|| self.env.def(d).members.get(name).map(|m| m.ty)),
            _ => None,
        }
    }

    pub fn assign_target(&mut self, t: &Expr, value: TypeId) {
        match &t.kind {
            ExprKind::Name(n) => match self.lookup_var(&n.text) {
                Some(v) => {
                    if self.vars[v.0 as usize].is_const {
                        self.report("E2010", n.span, format!("cannot assign to `{}`", n.text));
                    }
                    self.assign_var(v, value, n.span)
                }
                None => self.assign_global(n, value),
            },
            ExprKind::Field { obj, name } => {
                let o = self.infer(obj);
                self.assign_field(o, &name.text, name.span, value);
                // The field holds the assigned value now: its declared type,
                // narrowed by whether the value can be `nil`.
                if let Some((root, path)) = self.field_path(t) {
                    self.forget_fields(root, Some(&path));
                    if let Some(v) = self.field_var(t, true) {
                        let base = self.var_base(v);
                        if value == Types::NIL {
                            self.flow.insert(v, Types::NIL);
                        } else if value != Types::ANY && !self.env.types.is_optional(value) {
                            let non_nil = self.env.types.non_nil(base);
                            self.flow.insert(v, non_nil);
                        }
                    }
                }
            }
            ExprKind::Index { obj, key } => {
                let o = self.infer(obj);
                let k = self.infer(key);
                if let ExprKind::String(s) = &key.kind {
                    let name = String::from_utf8_lossy(&s.value).into_owned();
                    self.assign_field(o, &name, key.span, value);
                    return;
                }
                match self.env.types.get(o).clone() {
                    Ty::Array(e) => {
                        self.check_assignable(key.span, k, Types::NUMBER);
                        self.check_assignable(t.span, value, e);
                    }
                    Ty::Map(kt, vt) => {
                        self.check_assignable(key.span, k, kt);
                        let opt = self.env.types.optional(vt);
                        self.check_assignable(t.span, value, opt);
                    }
                    _ => {}
                }
            }
            _ => {
                self.infer(t);
            }
        }
    }

    fn assign_field(&mut self, o: TypeId, name: &str, span: Span, value: TypeId) {
        let members = self.env.types.members(o);
        for m in members {
            match self.env.types.get(m).clone() {
                Ty::Record(r) => match r.fields.get(name) {
                    Some(f) => {
                        if f.readonly {
                            self.report("E2010", span, format!("`{name}` is read-only"));
                        }
                        self.check_assignable(span, value, f.ty)
                    }
                    None => match r.indexer {
                        Some((_, v)) => self.check_assignable(span, value, v),
                        None if r.open => self.extend_open(m, name, value),
                        None if self.mode == super::query::Mode::Strict => {
                            self.report(
                                "E2051",
                                span,
                                format!("cannot add the field `{name}` to a sealed table"),
                            );
                        }
                        None => {}
                    },
                },
                Ty::Named(d, args) => match self.env.instance_member(d, &args, name) {
                    Some((mem, _)) => {
                        let in_ctor_or_class = self
                            .classes
                            .iter()
                            .any(|(cd, _)| *cd == d || self.env.derives(*cd, d));
                        if (mem.kind == MemberKind::Getter && mem.readonly)
                            || (mem.readonly && mem.kind == MemberKind::Field && !in_ctor_or_class)
                        {
                            self.report("E2010", span, format!("`{name}` is read-only"));
                        }
                        self.check_assignable(span, value, mem.ty);
                    }
                    None => {
                        if self.env.indexer_of(d).is_none() && self.env.def(d).kind != DefKind::Enum
                        {
                            if name == "__wowlua" {
                                self.report(
                                    "E2003",
                                    span,
                                    "`__wowlua` is reserved for the wowlua runtime",
                                );
                            } else if !self.env.def(d).native || self.env.def(d).widget.is_none() {
                                let msg =
                                    format!("`{}` has no field `{name}`", self.env.display(m));
                                self.report("E2003", span, msg);
                            }
                        } else if name == "__wowlua" {
                            self.report(
                                "E2003",
                                span,
                                "`__wowlua` is reserved for the wowlua runtime",
                            );
                        }
                    }
                },
                Ty::ClassOf(d) => {
                    if self.env.def(d).kind == DefKind::Enum {
                        self.report(
                            "E2010",
                            span,
                            format!("cannot assign to the enum member `{name}`"),
                        );
                    } else if let Some(mem) = self.env.static_member(d, name) {
                        self.check_assignable(span, value, mem.ty);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn compound_result(
        &mut self,
        op: CompoundOp,
        current: TypeId,
        value: TypeId,
        span: Span,
    ) -> TypeId {
        match op {
            CompoundOp::Concat => Types::STRING,
            CompoundOp::Coalesce => {
                let non_nil = self.env.types.non_nil(current);
                self.env.types.union(&[non_nil, value])
            }
            _ => {
                self.arith_operand(current, span);
                self.arith_operand(value, span);
                Types::NUMBER
            }
        }
    }

    // ----------------------------------------------------------- tables --

    fn table(&mut self, fields: &[TableField], expected: Option<TypeId>) -> TypeId {
        // The expected record, array or map gives field types.
        let exp = expected.map(|t| self.env.types.non_nil(t));
        // An expected union of table types (`Earned | Spent`): the first
        // member the literal fits.
        if let Some(e) = exp
            && let Ty::Union(members) = self.env.types.get(e).clone()
        {
            let tables: Vec<TypeId> = members
                .into_iter()
                .filter(|&m| {
                    matches!(
                        self.env.types.get(m),
                        Ty::Record(_) | Ty::Named(..) | Ty::Array(_) | Ty::Map(..)
                    )
                })
                .collect();
            if tables.len() >= 2 {
                for m in tables {
                    let mark = self.diagnostics.len();
                    let t = self.table(fields, Some(m));
                    if self.diagnostics.len() == mark && self.env.assignable(t, m, self.mode) {
                        return t;
                    }
                    self.diagnostics.truncate(mark);
                }
            }
        }
        let exp_ty = exp.map(|t| self.env.types.get(t).clone());
        let mut record = Record {
            open: exp.is_none(),
            ..Record::default()
        };
        let mut array: Vec<TypeId> = Vec::new();
        let mut keys: Vec<TypeId> = Vec::new();
        let mut values: Vec<TypeId> = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            match f {
                TableField::Named { name, value, .. } => {
                    let want =
                        exp.and_then(|t| self.field_type_quiet(t, &name.text))
                            .or(match &exp_ty {
                                Some(Ty::Map(_, v)) => Some(*v),
                                _ => None,
                            });
                    // Strict mode: a literal's unknown field for a record
                    // type is a typo (spec §2.2.4, E2003).
                    if self.mode == super::query::Mode::Strict
                        && let Some(Ty::Record(r)) = &exp_ty
                        && r.indexer.is_none()
                        && !r.fields.contains_key(name.text.as_ref())
                    {
                        let msg = format!(
                            "`{}` is not a field of `{}`",
                            name.text,
                            self.env.display(exp.expect("expected"))
                        );
                        self.report("E2003", name.span, msg);
                    }
                    let t = self.check(value, want);
                    if let Some(w) = want {
                        self.check_assignable(value.span, t, w);
                    }
                    record.fields.insert(
                        name.text.clone(),
                        Field {
                            // Literals stay only where the expected type
                            // gives the field a type.
                            ty: if want.is_some() {
                                t
                            } else {
                                self.env.types.widen(t)
                            },
                            optional: false,
                            readonly: false,
                        },
                    );
                }
                TableField::Keyed { key, value, .. } => {
                    let k = self.infer(key);
                    let want = match &exp_ty {
                        Some(Ty::Map(_, v)) => Some(*v),
                        Some(Ty::Array(e)) => Some(*e),
                        _ => None,
                    };
                    let v = self.check(value, want);
                    if let ExprKind::String(s) = &key.kind {
                        record.fields.insert(
                            String::from_utf8_lossy(&s.value).into(),
                            Field {
                                ty: self.env.types.widen(v),
                                optional: false,
                                readonly: false,
                            },
                        );
                    } else {
                        keys.push(self.env.types.widen(k));
                        values.push(self.env.types.widen(v));
                    }
                }
                TableField::Positional(value) => {
                    let want = match &exp_ty {
                        Some(Ty::Array(e)) => Some(*e),
                        Some(Ty::Map(_, v)) => Some(*v),
                        _ => None,
                    };
                    if i + 1 == fields.len() && is_multi(value) {
                        let p = self.expr_inner(value, want);
                        for t in p.types {
                            array.push(t);
                        }
                        if let Some(Rest::Variadic(t)) = p.rest {
                            array.push(t);
                        }
                    } else {
                        let t = self.check(value, want);
                        array.push(t);
                    }
                }
            }
        }
        if !keys.is_empty() {
            let k = self.env.types.union(&keys);
            let v = self.env.types.union(&values);
            record.indexer = match &exp_ty {
                Some(Ty::Map(ek, ev))
                    if self.env.assignable(k, *ek, self.mode)
                        && self.env.assignable(v, *ev, self.mode) =>
                {
                    Some((*ek, *ev))
                }
                _ => Some((k, v)),
            };
        }
        // The expected element type, when every element has it (arrays are
        // invariant in strict mode, so `"x"[]` would not be a `string[]`).
        let want_elem = match &exp_ty {
            Some(Ty::Array(e)) => Some(*e),
            Some(Ty::Map(_, v)) => Some(*v),
            _ => None,
        };
        let elem = if array.is_empty() {
            None
        } else if let Some(w) = want_elem
            && array.iter().all(|&t| self.env.assignable(t, w, self.mode))
        {
            Some(w)
        } else {
            let widened: Vec<TypeId> = array
                .iter()
                .map(|&t| {
                    if exp.is_some() {
                        t
                    } else {
                        self.env.types.widen(t)
                    }
                })
                .collect();
            Some(self.env.types.union(&widened))
        };
        // An empty literal takes the expected type (spec §2.4.3).
        if fields.is_empty()
            && let Some(t) = exp
            && !matches!(self.env.types.get(t), Ty::Any | Ty::Unknown | Ty::Error)
        {
            return t;
        }
        match (elem, record.fields.is_empty(), record.indexer) {
            (Some(e), true, None) => self.env.types.array(e),
            (Some(e), _, None) => {
                record.indexer = Some((Types::NUMBER, e));
                self.env.types.record(record)
            }
            _ => self.env.types.record(record),
        }
    }

    // ------------------------------------------------------- members --

    /// The type of `o.name`; unknown members of declared types are E2003.
    /// `quiet`: no diagnostics (for contextual lookups).
    pub fn member_type(
        &mut self,
        o: TypeId,
        name: &str,
        span: Span,
        quiet: bool,
    ) -> Option<TypeId> {
        let members = self.env.types.members(o);
        let mut out = Vec::new();
        for m in members {
            let t = match self.env.types.get(m).clone() {
                Ty::Any | Ty::Error | Ty::Table | Ty::Unknown => Some(Types::ANY),
                Ty::Nil => {
                    if self.mode == super::query::Mode::Strict && !quiet {
                        let msg = self.nil_message(o);
                        self.report("E2005", span, msg);
                    }
                    continue;
                }
                Ty::String | Ty::Str(_) => self.string_member(name),
                Ty::Record(r) => match r.fields.get(name) {
                    Some(f) => Some(f.ty),
                    None => match r.indexer {
                        Some((k, v)) => {
                            let key = self.env.types.str(name);
                            if self.env.assignable(key, k, super::query::Mode::Basic) {
                                Some(self.env.types.optional(v))
                            } else {
                                Some(Types::ANY)
                            }
                        }
                        // Fields of open tables may be assigned later.
                        None if r.open => Some(Types::ANY),
                        None if self.mode == super::query::Mode::Strict && !quiet => {
                            let msg = format!("`{}` has no field `{name}`", self.env.display(m));
                            self.report("E2003", span, msg);
                            Some(Types::ERROR)
                        }
                        None => Some(Types::ANY),
                    },
                },
                Ty::Array(e) => {
                    if name == "n" {
                        Some(Types::NUMBER)
                    } else {
                        let _ = e;
                        Some(Types::ANY)
                    }
                }
                Ty::Map(_, v) => Some(self.env.types.optional(v)),
                Ty::Named(d, args) => {
                    if self.env.def(d).kind == DefKind::Enum {
                        Some(Types::ANY)
                    } else {
                        match self.env.instance_member(d, &args, name) {
                            Some((mem, owner)) => {
                                if mem.renamable {
                                    self.private_methods.insert(span, owner);
                                }
                                if !quiet {
                                    self.check_visibility(mem.visibility, owner, name, span);
                                }
                                Some(match mem.kind {
                                    MemberKind::Method => self.env.with_self(mem.ty, m),
                                    _ => mem.ty,
                                })
                            }
                            None => match self.env.indexer_of(d) {
                                Some((_, v)) => Some(v),
                                None => {
                                    if !quiet {
                                        let msg = format!(
                                            "`{}` has no field or method `{name}`",
                                            self.env.display(m)
                                        );
                                        self.report("E2003", span, msg);
                                    }
                                    Some(Types::ERROR)
                                }
                            },
                        }
                    }
                }
                Ty::ClassOf(d) => self.static_type(d, name, span, quiet),
                Ty::EnumMember(..) => Some(Types::ANY),
                Ty::Module(_) => Some(Types::ANY),
                Ty::Never | Ty::Param(_) => Some(Types::ANY),
                // A callable table (`LibStub`): the first part with the member.
                Ty::Intersection(parts) => {
                    let found = parts.iter().find_map(|&p| match self.env.types.get(p) {
                        Ty::Func(_) => None,
                        _ => self
                            .member_type(p, name, span, true)
                            .filter(|&t| t != Types::ERROR),
                    });
                    Some(found.unwrap_or(Types::ANY))
                }
                Ty::Secret(_) => {
                    if !quiet {
                        self.check_secret(m, span);
                    }
                    Some(Types::ANY)
                }
                _ => {
                    if !quiet {
                        let msg = format!("cannot index a value of type `{}`", self.env.display(m));
                        self.report("E2003", span, msg);
                    }
                    Some(Types::ERROR)
                }
            };
            if let Some(t) = t {
                out.push(t);
            }
        }
        if out.is_empty() {
            return Some(Types::NIL);
        }
        Some(self.env.types.union(&out))
    }

    fn check_visibility(&mut self, v: Visibility, owner: DefId, name: &str, span: Span) {
        let inside = |c: &Checker, d: DefId| c.classes.iter().any(|(cd, _)| *cd == d);
        let ok = match v {
            Visibility::Public => true,
            Visibility::Private => inside(self, owner),
            Visibility::Protected => self
                .classes
                .iter()
                .any(|(cd, _)| *cd == owner || self.env.derives(*cd, owner)),
        };
        if !ok {
            let kind = if v == Visibility::Private {
                "private"
            } else {
                "protected"
            };
            self.report("E2011", span, format!("`{name}` is {kind}"));
        }
    }

    /// `C.name` for a class value, namespace or enum.
    fn static_type(&mut self, d: DefId, name: &str, span: Span, quiet: bool) -> Option<TypeId> {
        match self.env.def(d).kind {
            DefKind::Enum => {
                let idx = self
                    .env
                    .def(d)
                    .enum_members
                    .iter()
                    .position(|(n, _)| n.as_ref() == name);
                match idx {
                    Some(i) => Some(self.env.types.intern(Ty::EnumMember(d, i as u32))),
                    None => {
                        if !quiet {
                            let msg =
                                format!("enum `{}` has no member `{name}`", self.env.def(d).name);
                            self.report("E2003", span, msg);
                        }
                        Some(Types::ERROR)
                    }
                }
            }
            DefKind::Namespace => match self.env.def(d).members.get(name).map(|m| m.ty) {
                Some(t) => Some(t),
                None => {
                    if !quiet {
                        let msg = format!("`{}` has no member `{name}`", self.env.def(d).name);
                        self.report("E2003", span, msg);
                    }
                    Some(Types::ERROR)
                }
            },
            _ => match self.env.static_member(d, name) {
                Some(m) => {
                    if m.renamable {
                        self.private_methods.insert(span, d);
                    }
                    Some(m.ty)
                }
                None => {
                    // Instance methods through the class (`C.method(self, …)`).
                    let args: Vec<TypeId> =
                        self.env.def(d).params.iter().map(|_| Types::ANY).collect();
                    match self.env.instance_member(d, &args, name) {
                        Some((m, owner)) if m.kind == MemberKind::Method => {
                            if m.renamable {
                                self.private_methods.insert(span, owner);
                            }
                            let inst = self.env.instance(d);
                            Some(self.env.with_self(m.ty, inst))
                        }
                        _ => {
                            if !quiet && !self.env.def(d).native {
                                let msg = format!(
                                    "class `{}` has no static member `{name}`",
                                    self.env.def(d).name
                                );
                                self.report("E2003", span, msg);
                            }
                            Some(Types::ANY)
                        }
                    }
                }
            },
        }
    }

    /// `o:name(…)`: the method without `self`.
    fn method_type(&mut self, o: TypeId, name: &str, span: Span) -> TypeId {
        let mut out = Vec::new();
        for m in self.env.types.members(o) {
            let t = match self.env.types.get(m).clone() {
                Ty::Nil => {
                    if self.mode == super::query::Mode::Strict {
                        let msg = self.nil_message(o);
                        self.report("E2005", span, msg);
                    }
                    continue;
                }
                Ty::String | Ty::Str(_) => self
                    .string_member(name)
                    .map(|f| self.drop_first(f))
                    .unwrap_or(Types::ANY),
                Ty::Named(d, args) if self.env.def(d).kind != DefKind::Enum => {
                    match self.env.instance_member(d, &args, name) {
                        Some((mem, owner)) => {
                            if mem.renamable {
                                self.private_methods.insert(span, owner);
                            }
                            self.check_visibility(mem.visibility, owner, name, span);
                            match mem.kind {
                                MemberKind::Method => mem.ty,
                                // A function field called with `:` gets the
                                // object as its first argument.
                                _ => self.drop_first(mem.ty),
                            }
                        }
                        None => match self.env.indexer_of(d) {
                            Some(_) => Types::ANY,
                            None => {
                                let msg =
                                    format!("`{}` has no method `{name}`", self.env.display(m));
                                self.report("E2003", span, msg);
                                Types::ERROR
                            }
                        },
                    }
                }
                _ => {
                    let f = self.member_type(m, name, span, false).unwrap_or(Types::ANY);
                    self.drop_first(f)
                }
            };
            out.push(t);
        }
        if out.is_empty() {
            return Types::ANY;
        }
        self.env.types.union(&out)
    }

    /// A function type without its first parameter (`self`).
    fn drop_first(&mut self, t: TypeId) -> TypeId {
        match self.env.types.get(t).clone() {
            // `F?`: each function member (`nil` stays, reported by the call).
            Ty::Union(members) => {
                let out: Vec<TypeId> = members.into_iter().map(|m| self.drop_first(m)).collect();
                self.env.types.union(&out)
            }
            Ty::Func(mut f) => {
                if !f.params.is_empty() {
                    f.params.remove(0);
                } else if matches!(f.rest, Some(Rest::Variadic(_))) {
                    // `(...any)` stays.
                } else {
                    return t;
                }
                if let Some((i, p)) = f.predicate {
                    f.predicate = i.checked_sub(1).map(|i| (i, p));
                }
                self.env.types.func(f)
            }
            Ty::Intersection(ms) => {
                let ms: Vec<TypeId> = ms.iter().map(|&x| self.drop_first(x)).collect();
                self.env.types.intern(Ty::Intersection(ms))
            }
            _ => t,
        }
    }

    /// `s:upper()` is `string.upper(s)` (spec §2.4.5).
    fn string_member(&mut self, name: &str) -> Option<TypeId> {
        let string_ns = self.env.globals.get("string").copied()?;
        match self.env.types.get(string_ns).clone() {
            Ty::ClassOf(d) => self
                .env
                .def(d)
                .members
                .get(name)
                .map(|m| m.ty)
                .or(Some(Types::ANY)),
            Ty::Record(r) => r.fields.get(name).map(|f| f.ty).or(Some(Types::ANY)),
            _ => Some(Types::ANY),
        }
    }

    fn index_type(&mut self, o: TypeId, k: TypeId, key: &Expr, span: Span) -> TypeId {
        if let ExprKind::String(s) = &key.kind {
            let name = String::from_utf8_lossy(&s.value).into_owned();
            return self
                .member_type(o, &name, key.span, false)
                .unwrap_or(Types::ANY);
        }
        self.check_secret(k, key.span);
        let mut out = Vec::new();
        for m in self.env.types.members(o) {
            let t = match self.env.types.get(m).clone() {
                Ty::Array(e) => {
                    self.check_assignable(key.span, k, Types::NUMBER);
                    // Array reads are `T` in every mode, although reading past
                    // the end gives `nil` (decision T95).
                    e
                }
                Ty::Map(_, v) => self.env.types.optional(v),
                Ty::Record(r) => match r.indexer {
                    Some((_, v)) => self.env.types.optional(v),
                    None => Types::ANY,
                },
                Ty::Nil => {
                    if self.mode == super::query::Mode::Strict {
                        let msg = self.nil_message(o);
                        self.report("E2005", span, msg);
                    }
                    continue;
                }
                Ty::Named(d, _) => match self.env.indexer_of(d) {
                    Some((_, v)) => v,
                    None => Types::ANY,
                },
                Ty::Secret(_) => {
                    self.check_secret(m, span);
                    Types::ANY
                }
                _ => Types::ANY,
            };
            out.push(t);
        }
        if out.is_empty() {
            return Types::NIL;
        }
        self.env.types.union(&out)
    }

    // --------------------------------------------------------- operators --

    fn binary(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        expected: Option<TypeId>,
    ) -> TypeId {
        match op {
            BinOp::And => {
                self.mark_probes(lhs);
                let l = self.check(lhs, expected);
                let (t_flow, f_flow) = self.narrow(lhs, &self.flow.clone());
                let (t_guard, _) = self.narrow_guard(lhs, &self.guard.clone());
                let saved = std::mem::replace(&mut self.flow, t_flow);
                let saved_guard = std::mem::replace(&mut self.guard, t_guard);
                let r = self.check(rhs, expected);
                self.flow = saved;
                self.guard = saved_guard;
                let _ = f_flow;
                let lf = self.falsy(l);
                self.env.types.union(&[lf, r])
            }
            BinOp::Or => {
                self.mark_probes(lhs);
                let l = self.check(lhs, expected);
                let (_, f_flow) = self.narrow(lhs, &self.flow.clone());
                let (_, f_guard) = self.narrow_guard(lhs, &self.guard.clone());
                let saved = std::mem::replace(&mut self.flow, f_flow);
                let saved_guard = std::mem::replace(&mut self.guard, f_guard);
                let r = self.check(rhs, expected);
                self.flow = saved;
                self.guard = saved_guard;
                let lt = self.truthy(l);
                self.env.types.union(&[lt, r])
            }
            BinOp::Coalesce => {
                let l = self.check(lhs, expected);
                let r = self.check(rhs, expected);
                let non_nil = self.env.types.non_nil(l);
                self.env.types.union(&[non_nil, r])
            }
            BinOp::Eq | BinOp::Ne => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                self.check_secret(l, lhs.span);
                self.check_secret(r, rhs.span);
                Types::BOOLEAN
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                self.check_secret(l, lhs.span);
                self.check_secret(r, rhs.span);
                let ln = self.env.assignable(l, Types::NUMBER, self.mode);
                let ls = self.env.assignable(l, Types::STRING, self.mode);
                let rn = self.env.assignable(r, Types::NUMBER, self.mode);
                let rs = self.env.assignable(r, Types::STRING, self.mode);
                if !((ln && rn) || (ls && rs)) && !self.has_metamethods(l) {
                    let msg = format!(
                        "cannot compare `{}` with `{}`",
                        self.env.display(l),
                        self.env.display(r)
                    );
                    self.report("E2001", span, msg);
                }
                Types::BOOLEAN
            }
            BinOp::Concat => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                for (t, e) in [(l, lhs), (r, rhs)] {
                    // Every member must be a string, number or secret.
                    let members = self.env.types.members(t);
                    let ok = members.iter().all(|&m| {
                        self.env.assignable(m, Types::STRING, self.mode)
                            || self.env.assignable(m, Types::NUMBER, self.mode)
                            || matches!(self.env.types.get(m), Ty::Secret(_))
                    }) || self.has_metamethods(t);
                    if !ok {
                        let msg = format!(
                            "cannot concatenate a value of type `{}`",
                            self.env.display(t)
                        );
                        self.report("E2001", e.span, msg);
                    }
                }
                if self.is_secret(l) || self.is_secret(r) {
                    let secret = self.env.types.intern(Ty::Secret(Types::STRING));
                    return self.env.types.union(&[Types::STRING, secret]);
                }
                Types::STRING
            }
            _ => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                self.check_secret(l, lhs.span);
                self.check_secret(r, rhs.span);
                self.arith_operand(l, lhs.span);
                self.arith_operand(r, rhs.span);
                Types::NUMBER
            }
        }
    }

    /// Classes and tables with metatables may define operators.
    fn has_metamethods(&self, t: TypeId) -> bool {
        self.env.types.members(t).iter().any(|&m| {
            matches!(
                self.env.types.get(m),
                Ty::Named(..)
                    | Ty::Table
                    | Ty::Record(_)
                    | Ty::Any
                    | Ty::Error
                    | Ty::Unknown
                    | Ty::Param(_)
            )
        })
    }

    /// Arithmetic needs numbers (strings coerce in Lua).
    pub fn arith_operand(&mut self, t: TypeId, span: Span) {
        let ok = self.env.assignable(t, Types::NUMBER, self.mode)
            || self.env.assignable(t, Types::STRING, self.mode)
            || self.has_metamethods(t)
            || self.is_secret(t);
        if !ok {
            let msg = format!("arithmetic on a value of type `{}`", self.env.display(t));
            self.report("E2001", span, msg);
        }
    }

    pub fn is_secret(&self, t: TypeId) -> bool {
        self.env
            .types
            .members(t)
            .iter()
            .any(|&m| matches!(self.env.types.get(m), Ty::Secret(_)))
    }

    /// A forbidden operation on a possibly secret value (spec §2.11).
    pub fn check_secret(&mut self, t: TypeId, span: Span) {
        if self.is_secret(t) {
            let code = if self.mode == super::query::Mode::Strict {
                "E2040"
            } else {
                "W2040"
            };
            let msg = format!(
                "`{}` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first",
                self.env.display(t)
            );
            self.report(code, span, msg);
        }
    }

    // -------------------------------------------------------------- calls --

    fn args_types(&mut self, args: &Args, expected: &[Option<TypeId>]) -> Pack {
        match args {
            Args::Parens { args, .. } => self.expr_pack(args, expected),
            Args::Table(e) | Args::String(e) => {
                Pack::single(self.check(e, expected.first().copied().flatten()))
            }
        }
    }

    fn arg_exprs(args: &Args) -> Vec<&Expr> {
        args.exprs()
    }

    /// A call of `callee` (with `receiver` for `:` calls on non-method
    /// members already handled). Returns the results.
    /// `coroutine.yield` inside a function passed to `pcall`, `xpcall` or
    /// a C callback cannot yield across that boundary (W2057).
    fn check_yield_boundary(&mut self, callee: &Expr, args: &Args) {
        let name = match &callee.kind {
            ExprKind::Name(n) => n.text.to_string(),
            ExprKind::Field { obj, name } => match &obj.kind {
                ExprKind::Name(o) => format!("{}.{}", o.text, name.text),
                _ => return,
            },
            _ => return,
        };
        if !matches!(
            name.as_str(),
            "pcall" | "xpcall" | "table.sort" | "sort" | "string.gsub" | "gsub"
        ) {
            return;
        }
        for a in args.exprs() {
            let ExprKind::Function(f) = &a.kind else {
                continue;
            };
            let mut found = None;
            for s in &f.body.stmts {
                wowlua_syntax::visit::walk_stmt(s, &mut |e| {
                    if found.is_none()
                        && let ExprKind::Call { callee, .. } = &e.kind
                        && let ExprKind::Field { obj, name } = &callee.kind
                        && name.text.as_ref() == "yield"
                        && matches!(&obj.kind, ExprKind::Name(c) if c.text.as_ref() == "coroutine")
                    {
                        found = Some(e.span);
                    }
                });
            }
            if let Some(span) = found {
                self.report(
                    "W2057",
                    span,
                    format!("`coroutine.yield` cannot yield across `{name}` in Lua 5.1"),
                );
            }
        }
    }

    pub fn call(
        &mut self,
        callee: TypeId,
        receiver: Option<TypeId>,
        args: &Args,
        span: Span,
        expected: Option<TypeId>,
    ) -> Pack {
        let _ = receiver;
        let members = self.env.types.members(callee);
        if members.len() > 1 {
            // A union of callables: each must accept the call.
            let mut results = Vec::new();
            let mut first = true;
            for m in members {
                if m == Types::NIL {
                    if self.mode == super::query::Mode::Strict {
                        let msg = self.nil_message(callee);
                        self.report("E2005", span, msg);
                    }
                    continue;
                }
                let p = if first {
                    self.call_one(m, args, span, expected)
                } else {
                    self.call_quiet(m, args)
                };
                first = false;
                results.push(p.types.first().copied().unwrap_or(Types::NIL));
            }
            let u = self.env.types.union(&results);
            return Pack::single(u);
        }
        self.call_one(callee, args, span, expected)
    }

    /// The results of a call without checking (other union members).
    fn call_quiet(&mut self, callee: TypeId, _args: &Args) -> Pack {
        match self.env.types.get(callee).clone() {
            Ty::Func(f) => f.ret,
            _ => Pack::any(),
        }
    }

    fn call_one(
        &mut self,
        callee: TypeId,
        args: &Args,
        span: Span,
        expected: Option<TypeId>,
    ) -> Pack {
        match self.env.types.get(callee).clone() {
            Ty::Func(f) => self
                .call_func(&f, args, span, expected, true)
                .unwrap_or_else(Pack::any),
            Ty::Intersection(overloads) => self.call_overloads(&overloads, args, span, expected),
            Ty::ClassOf(d) => {
                if self.env.def(d).kind == DefKind::Class {
                    if self.env.def(d).is_abstract {
                        let msg = format!(
                            "cannot instantiate the abstract class `{}`",
                            self.env.def(d).name
                        );
                        self.report("E3002", span, msg);
                    }
                    match self.env.static_member(d, "new").map(|m| m.ty) {
                        Some(new) => self.call_one(new, args, span, expected),
                        None => {
                            self.args_types(args, &[]);
                            Pack::any()
                        }
                    }
                } else {
                    self.args_types(args, &[]);
                    Pack::any()
                }
            }
            Ty::Any
            | Ty::Error
            | Ty::Function
            | Ty::Table
            | Ty::Named(..)
            | Ty::Record(_)
            | Ty::Never
            | Ty::Param(_) => {
                self.args_types(args, &[]);
                Pack::any()
            }
            Ty::Unknown => {
                self.args_types(args, &[]);
                self.report(
                    "E2004",
                    span,
                    "`unknown` must be narrowed before it is called",
                );
                Pack::any()
            }
            Ty::Secret(_) => {
                self.args_types(args, &[]);
                self.check_secret(callee, span);
                Pack::any()
            }
            _ => {
                self.args_types(args, &[]);
                let msg = format!(
                    "a value of type `{}` is not callable",
                    self.env.display(callee)
                );
                self.report("E2004", span, msg);
                Pack::any()
            }
        }
    }

    /// Overload resolution (spec §2.7): the first overload whose parameters
    /// accept the non-function arguments.
    fn call_overloads(
        &mut self,
        overloads: &[TypeId],
        args: &Args,
        span: Span,
        expected: Option<TypeId>,
    ) -> Pack {
        let exprs = Self::arg_exprs(args);
        // Argument types without contextual typing (function expressions
        // match any function type).
        let saved_diags = self.diagnostics.len();
        let arg_types: Vec<TypeId> = exprs
            .iter()
            .map(|e| {
                if matches!(e.kind, ExprKind::Function(_)) {
                    Types::FUNCTION
                } else {
                    self.peek_or_check(e)
                }
            })
            .collect();
        self.diagnostics.truncate(saved_diags);
        for &o in overloads {
            let Ty::Func(f) = self.env.types.get(o).clone() else {
                continue;
            };
            if self.accepts(&f, &arg_types) {
                return self
                    .call_func(&f, args, span, expected, true)
                    .unwrap_or_else(Pack::any);
            }
        }
        // No overload matches (E2012).
        self.args_types(args, &[]);
        let list: Vec<String> = overloads.iter().map(|&o| self.env.display(o)).collect();
        let got: Vec<String> = arg_types.iter().map(|&t| self.env.display(t)).collect();
        let msg = format!(
            "no overload matches the arguments ({}); overloads:\n  {}",
            got.join(", "),
            list.join("\n  ")
        );
        self.report("E2012", span, msg);
        Pack::any()
    }

    fn peek_or_check(&mut self, e: &Expr) -> TypeId {
        match self.expr_types.get(&(e.span.start, e.span.end)) {
            Some(&t) => t,
            None => {
                // A check without lasting effects on the flow.
                let saved = self.flow.clone();
                let t = self.infer(e);
                self.flow = saved;
                t
            }
        }
    }

    /// Whether `f` accepts arguments of these types (overload filtering).
    fn accepts(&mut self, f: &Func, args: &[TypeId]) -> bool {
        let f = self.instantiate_any(f);
        for (i, &a) in args.iter().enumerate() {
            let p = match f.params.get(i) {
                Some(p) => p.ty,
                None => match &f.rest {
                    Some(Rest::Variadic(t)) => *t,
                    Some(Rest::Pack(_)) => Types::ANY,
                    None => return false,
                },
            };
            if a == Types::FUNCTION
                && matches!(
                    self.env.types.get(p),
                    Ty::Func(_) | Ty::Function | Ty::Any | Ty::Union(_)
                )
            {
                continue;
            }
            if !self.env.assignable(a, p, self.mode) {
                return false;
            }
        }
        f.params.iter().skip(args.len()).all(|p| p.optional)
    }

    /// `f` with its type parameters as `any`.
    fn instantiate_any(&mut self, f: &Func) -> Func {
        if f.type_params.is_empty() {
            return f.clone();
        }
        let mut s = Subst::default();
        for p in &f.type_params {
            if self.env.params[p.0 as usize].pack {
                s.packs.insert(*p, Pack::any());
            } else {
                s.types.insert(*p, Types::ANY);
            }
        }
        subst::apply_func(&mut self.env.types, f, &s)
    }

    /// `saved.bind(name, { defaults = … })` returns the data table: the
    /// type of the defaults (spec stdlib "saved").
    fn saved_bind_type(&mut self, callee: &Expr, args: &Args) -> Option<TypeId> {
        if self.intrinsic_of(callee).as_deref() != Some("saved.bind") {
            return None;
        }
        let ExprKind::Table(fields) = &args.exprs().get(1)?.kind else {
            return None;
        };
        fields.iter().find_map(|f| match f {
            TableField::Named { name, value, .. } if name.text.as_ref() == "defaults" => self
                .expr_types
                .get(&(value.span.start, value.span.end))
                .copied(),
            _ => None,
        })
    }

    /// `L.KEY(params)` of `wowlua:locale` with a value that may be secret:
    /// the string made from it is secret too (spec stdlib "locale").
    fn secret_locale_call(&mut self, callee: &Expr, args: &Args) -> bool {
        let ExprKind::Field { obj, .. } = &callee.kind else {
            return false;
        };
        if self.intrinsic_of(obj).as_deref() != Some("locale.L") {
            return false;
        }
        args.exprs().first().is_some_and(|a| {
            self.expr_types
                .get(&(a.span.start, a.span.end))
                .is_some_and(|&t| {
                    self.secret_path(t, String::new(), &mut std::collections::HashSet::new())
                        .is_some()
                })
        })
    }

    /// Checks of standard-library calls that need their arguments' types:
    /// `Secret<…>` in `saved.bind` defaults (E8104), hook targets (W8040).
    fn stdlib_after_call(&mut self, callee: &Expr, args: &Args) {
        let Some(id) = self.intrinsic_of(callee) else {
            return;
        };
        if id.starts_with("hooks.") {
            self.check_hook_target(&id, &args.exprs(), false);
        }
        if id == "settings.register"
            && let Some(def) = args.exprs().first()
        {
            self.check_settings(def);
        }
        if id == "saved.bind"
            && let Some(opts) = args.exprs().get(1)
            && let ExprKind::Table(fields) = &opts.kind
        {
            for f in fields {
                if let TableField::Named { name, value, .. } = f
                    && name.text.as_ref() == "defaults"
                    && let Some(&t) = self.expr_types.get(&(value.span.start, value.span.end))
                    && let Some(path) =
                        self.secret_path(t, String::new(), &mut std::collections::HashSet::new())
                {
                    let at = if path.is_empty() {
                        String::new()
                    } else {
                        format!(" (at `{path}`)")
                    };
                    let msg = format!(
                        "the defaults of `saved.bind` contain a `Secret<…>` type{at}: secret values cannot be compared or saved"
                    );
                    self.report("E8104", value.span, msg);
                }
            }
        }
    }

    /// `settings.register({ store = …, controls = { … } })`: every control's
    /// key is a field of the store (E8113) of the control's type (E8070),
    /// and dropdown values are of the field's type (E8107).
    fn check_settings(&mut self, def: &Expr) {
        let ExprKind::Table(fields) = &def.kind else {
            return;
        };
        let field = |name: &str| {
            fields.iter().find_map(|f| match f {
                TableField::Named { name: n, value, .. } if n.text.as_ref() == name => Some(value),
                _ => None,
            })
        };
        let Some(store) = field("store") else { return };
        let Some(&store_type) = self.expr_types.get(&(store.span.start, store.span.end)) else {
            return;
        };
        if matches!(
            self.env.types.get(store_type),
            Ty::Any | Ty::Unknown | Ty::Error | Ty::Table
        ) {
            return;
        }
        let mut lists: Vec<&Expr> = field("controls").into_iter().collect();
        if let Some(subs) = field("subcategories")
            && let ExprKind::Table(entries) = &subs.kind
        {
            for e in entries {
                if let TableField::Positional(sub) = e
                    && let ExprKind::Table(sub_fields) = &sub.kind
                {
                    lists.extend(sub_fields.iter().filter_map(|f| match f {
                        TableField::Named { name, value, .. }
                            if name.text.as_ref() == "controls" =>
                        {
                            Some(value)
                        }
                        _ => None,
                    }));
                }
            }
        }
        for list in lists {
            let ExprKind::Table(entries) = &list.kind else {
                continue;
            };
            for e in entries {
                let TableField::Positional(control) = e else {
                    continue;
                };
                if let ExprKind::Call { callee, args } = &control.kind {
                    self.check_settings_control(store_type, callee, &args.exprs());
                }
            }
        }
    }

    fn check_settings_control(&mut self, store: TypeId, callee: &Expr, args: &[&Expr]) {
        let Some(id) = self.intrinsic_of(callee) else {
            return;
        };
        let kind = match id.as_str() {
            "settings.checkbox" => "checkbox",
            "settings.slider" => "slider",
            "settings.dropdown" => "dropdown",
            "settings.color" => "color",
            _ => return,
        };
        let Some(&key_expr) = args.first() else {
            return;
        };
        let ExprKind::String(lit) = &key_expr.kind else {
            return;
        };
        let key = String::from_utf8_lossy(&lit.value).into_owned();
        let found = match self.env.types.get(store).clone() {
            Ty::Named(d, targs) => self.env.instance_member(d, &targs, &key).map(|(m, _)| m.ty),
            Ty::Record(r) => r.fields.get(key.as_str()).map(|f| f.ty),
            _ => return,
        };
        let Some(field) = found else {
            let msg = format!(
                "the store `{}` has no field `{key}`",
                self.env.display(store)
            );
            self.report("E8113", key_expr.span, msg);
            return;
        };
        let value = self.env.types.non_nil(field);
        if matches!(self.env.types.get(value), Ty::Any | Ty::Unknown | Ty::Error) {
            return;
        }
        let both = |c: &mut Self, t: TypeId| {
            c.env.assignable(value, t, c.mode) && c.env.assignable(t, field, c.mode)
        };
        let fits = match kind {
            "checkbox" => both(self, Types::BOOLEAN),
            "slider" => both(self, Types::NUMBER),
            "color" => ["r", "g", "b"].iter().all(|c| {
                matches!(self.member_type(value, c, key_expr.span, true), Some(t)
                    if t != Types::ERROR && self.env.assignable(t, Types::NUMBER, self.mode))
            }),
            _ => true,
        };
        if !fits {
            let wanted = match kind {
                "checkbox" => "`boolean`",
                "slider" => "`number`",
                _ => "`{ r: number, g: number, b: number, a: number? }`",
            };
            let msg = format!(
                "`{key}` is `{}`; a {kind} needs {wanted}",
                self.env.display(field)
            );
            self.report("E8070", key_expr.span, msg);
            return;
        }
        if kind == "dropdown"
            && let Some(opts) = args.get(1)
            && let ExprKind::Table(opt_fields) = &opts.kind
        {
            for f in opt_fields {
                let TableField::Named {
                    name,
                    value: options,
                    ..
                } = f
                else {
                    continue;
                };
                if name.text.as_ref() != "options" {
                    continue;
                }
                let ExprKind::Table(entries) = &options.kind else {
                    continue;
                };
                for entry in entries {
                    let TableField::Positional(option) = entry else {
                        continue;
                    };
                    let ExprKind::Table(parts) = &option.kind else {
                        continue;
                    };
                    for p in parts {
                        if let TableField::Named { name, value: v, .. } = p
                            && name.text.as_ref() == "value"
                            && let Some(&t) = self.expr_types.get(&(v.span.start, v.span.end))
                            && !self.env.assignable(t, field, self.mode)
                        {
                            let msg = format!(
                                "the option value `{}` is not a `{}` (the type of `{key}`)",
                                self.env.display(t),
                                self.env.display(field)
                            );
                            self.report("E8107", v.span, msg);
                        }
                    }
                }
            }
        }
    }

    /// W8102: the target belongs to a load-on-demand addon, but the hook
    /// does not wait for it (`opts.addon`), or names it by an expression
    /// (evaluated before the addon loads).
    fn check_hook_on_demand(&mut self, id: &str, args: &[&Expr], decorator: bool) {
        let Some(&first) = args.first() else { return };
        let (root, by_name) = match &first.kind {
            ExprKind::String(lit) => (String::from_utf8_lossy(&lit.value).into_owned(), true),
            ExprKind::Name(n) if self.lookup_var(&n.text).is_none() => (n.text.to_string(), false),
            _ => return,
        };
        let Some(addon) = self.env.on_demand.get(root.as_str()).cloned() else {
            return;
        };
        let has_method = matches!(args.get(1).map(|e| &e.kind), Some(ExprKind::String(_)));
        let global_only = id == "hooks.hook" && !has_method;
        let opts_at = if decorator { 2 } else { 3 };
        let waits = !global_only && match args.get(opts_at).map(|e| &e.kind) {
            Some(ExprKind::Table(fields)) => fields.iter().any(
                |f| matches!(f, TableField::Named { name, .. } if name.text.as_ref() == "addon"),
            ),
            // Options from elsewhere: cannot tell.
            Some(_) => true,
            None => false,
        };
        let msg = if global_only {
            format!(
                "`{root}` exists only once the load-on-demand addon `{addon}` has loaded; hook it with `hook(_G, \"{root}\", fn, {{ addon = \"{addon}\" }})`"
            )
        } else if !waits {
            format!(
                "`{root}` exists only once the load-on-demand addon `{addon}` has loaded; pass `{{ addon = \"{addon}\" }}` so the hook waits for it"
            )
        } else if !by_name {
            format!(
                "`{root}` is read before `{addon}` loads; name it as a string (\"{root}\") so the hook finds it once the addon has loaded"
            )
        } else {
            return;
        };
        self.report("W8102", first.span, msg);
    }

    /// A hook's target exists (W8040): `hook(name, fn)` and `@hook(name)`
    /// name a global function, `hook(target, method, …)` a method of the
    /// target, `hookScript(frame, …)` a frame. A target of a load-on-demand
    /// addon needs `opts.addon` and a name (W8102).
    pub(crate) fn check_hook_target(&mut self, id: &str, args: &[&Expr], decorator: bool) {
        let literal = |e: &Expr| match &e.kind {
            ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value).into_owned()),
            _ => None,
        };
        self.check_hook_on_demand(id, args, decorator);
        let known = |c: &Self, name: &str| {
            c.env.globals.contains_key(name)
                || c.extra_globals.contains(name)
                || c.assigned_globals.contains(name)
                || c.dynamic_env
        };
        let Some(&first) = args.first() else { return };
        let method = args.get(1).and_then(|e| literal(e));
        let global_only = id == "hooks.hook" && method.is_none();
        if global_only || id == "hooks.hookScript" {
            if let Some(name) = literal(first)
                && !known(self, &name)
            {
                let what = if global_only { "function" } else { "frame" };
                let msg = format!(
                    "`{name}` is not a declared global {what}; the hook fails where it is installed"
                );
                self.report("W8040", first.span, msg);
            }
            return;
        }
        let (Some(method), Some(&method_expr)) = (method, args.get(1)) else {
            return;
        };
        let target = match literal(first) {
            Some(name) => match self.env.globals.get(name.as_str()) {
                Some(&t) => t,
                None => {
                    if !known(self, &name) {
                        let msg = format!(
                            "`{name}` is not a declared global; the hook fails where it is installed"
                        );
                        self.report("W8040", first.span, msg);
                    }
                    return;
                }
            },
            None => match self.expr_types.get(&(first.span.start, first.span.end)) {
                Some(&t) => t,
                None => self.infer(first),
            },
        };
        if matches!(
            self.env.types.get(target),
            Ty::Any | Ty::Unknown | Ty::Error | Ty::Table
        ) {
            return;
        }
        // Declared members (native widget types are open to any field, so
        // a member lookup would always succeed).
        let missing = match self.env.types.get(target).clone() {
            Ty::Named(d, targs) => {
                self.env.def(d).kind != DefKind::Enum
                    && self.env.instance_member(d, &targs, &method).is_none()
                    && self.env.def(d).indexer.is_none()
            }
            _ => matches!(
                self.member_type(target, &method, method_expr.span, true),
                None | Some(Types::ERROR)
            ),
        };
        if missing {
            let msg = format!(
                "`{}` has no `{method}` to hook; the hook fails where it is installed",
                self.env.display(target)
            );
            self.report("W8040", method_expr.span, msg);
        }
    }

    /// Where `t` contains a `Secret<…>` type: its field path, if any.
    fn secret_path(
        &self,
        t: TypeId,
        path: String,
        seen: &mut std::collections::HashSet<TypeId>,
    ) -> Option<String> {
        if !seen.insert(t) {
            return None;
        }
        let join = |name: &str| {
            if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}.{name}")
            }
        };
        match self.env.types.get(t).clone() {
            Ty::Secret(_) => Some(path),
            Ty::Union(ms) | Ty::Intersection(ms) => ms
                .iter()
                .find_map(|&m| self.secret_path(m, path.clone(), seen)),
            Ty::Array(e) => self.secret_path(e, join("[]"), seen),
            Ty::Map(k, v) => self
                .secret_path(k, join("[key]"), seen)
                .or_else(|| self.secret_path(v, join("[]"), seen)),
            Ty::Record(r) => r
                .fields
                .iter()
                .find_map(|(n, f)| self.secret_path(f.ty, join(n), seen))
                .or_else(|| {
                    r.indexer
                        .and_then(|(_, v)| self.secret_path(v, join("[]"), seen))
                }),
            Ty::Named(d, _) => {
                let def = self.env.def(d);
                let members: Vec<(Box<str>, TypeId)> =
                    def.members.iter().map(|(n, m)| (n.clone(), m.ty)).collect();
                let indexer = def.indexer;
                members
                    .iter()
                    .find_map(|(n, ty)| self.secret_path(*ty, join(n), seen))
                    .or_else(|| indexer.and_then(|(_, v)| self.secret_path(v, join("[]"), seen)))
            }
            _ => None,
        }
    }

    /// `FramePool.new(cls, ...)`: the rest of the arguments are `cls`'s
    /// constructor's, and the pool holds `cls`'s instances.
    fn frame_pool_type(&mut self, mut f: Func, exprs: &[&Expr]) -> Option<TypeId> {
        let first = exprs.first()?;
        if !matches!(first.kind, ExprKind::Name(_) | ExprKind::Field { .. }) {
            return None;
        }
        let cls = self.infer(first);
        let Ty::ClassOf(d) = self.env.types.get(cls).clone() else {
            return None;
        };
        if self.env.def(d).kind != DefKind::Class {
            return None;
        }
        let ctor = self.env.def(d).statics.get("new").map(|m| m.ty)?;
        let Ty::Func(ctor) = self.env.types.get(ctor).clone() else {
            return None;
        };
        let targs = vec![Types::ANY; self.env.def(d).params.len()];
        let instance = self.env.types.intern(Ty::Named(d, targs));
        let pool = f.ret.types.first().copied()?;
        let Ty::Named(pool_def, _) = self.env.types.get(pool).clone() else {
            return None;
        };
        f.params.truncate(1);
        f.params.extend(ctor.params);
        f.rest = ctor.rest;
        f.ret = Pack::single(self.env.types.intern(Ty::Named(pool_def, vec![instance])));
        Some(self.env.types.func(f))
    }

    /// A standard-library call typed by its event or message: the handler
    /// of `on`, `once` and `onUnit` gets the event's parameters, and `send`
    /// takes the message's.
    /// `registry:on/once/fire(name, …)` on a `callbacks.Registry<E>`: `name`
    /// must be a literal key of `E` (E8420); the handler, or the arguments,
    /// are checked against `E[name]` (spec stdlib "callbacks").
    fn registry_method_type(&mut self, o: TypeId, method: &str, args: &Args, m: TypeId) -> TypeId {
        if !matches!(method, "on" | "once" | "fire") {
            return m;
        }
        let Ty::Named(d, targs) = self.env.types.get(o).clone() else {
            return m;
        };
        if self.env.callbacks_registry != Some(d) {
            return m;
        }
        let Some(&events) = targs.first() else {
            return m;
        };
        if events == Types::ANY || matches!(self.env.types.get(events), Ty::Param(_)) {
            return m;
        }
        let exprs = args.exprs();
        let Some(first) = exprs.first() else {
            return m;
        };
        let ExprKind::String(lit) = &first.kind else {
            let msg = format!(
                "the event name of `{method}` must be a string literal naming a key of `{}`",
                self.env.display(events)
            );
            self.report("E8420", first.span, msg);
            return m;
        };
        let event = String::from_utf8_lossy(&lit.value).into_owned();
        let Some(handler) = self.field_type_quiet(events, &event) else {
            let msg = format!(
                "`{event}` is not an event of `{}`",
                self.env.display(events)
            );
            self.report("E8420", first.span, msg);
            return m;
        };
        let (Ty::Func(mut f), Some(h)) = (self.env.types.get(m).clone(), self.as_func(handler))
        else {
            return m;
        };
        if method == "fire" {
            f.params.truncate(1);
            f.params.extend(h.params);
            f.rest = h.rest;
        } else if let Some(p) = f.params.get_mut(1) {
            p.ty = handler;
        }
        self.env.types.func(f)
    }

    fn stdlib_call_type(&mut self, callee: &Expr, args: &Args, c: TypeId) -> TypeId {
        let Some(id) = self.intrinsic_of(callee) else {
            return c;
        };
        let Ty::Func(mut f) = self.env.types.get(c).clone() else {
            return c;
        };
        let exprs = args.exprs();
        if id == "pool.FramePool.new" {
            return self.frame_pool_type(f, &exprs).unwrap_or(c);
        }
        let (name, name_span) = match exprs.first().map(|e| (&e.kind, e.span)) {
            Some((ExprKind::String(lit), span)) => {
                (String::from_utf8_lossy(&lit.value).into_owned(), span)
            }
            _ => return c,
        };
        if matches!(id.as_str(), "events.on" | "events.once" | "events.onUnit") {
            self.check_event_available(&name, name_span);
        }
        let Some(handler) = self.env.handlers.get(name.as_str()).cloned() else {
            return c;
        };
        match id.as_str() {
            "events.on" | "events.once" | "events.onUnit" => {
                let at = if id == "events.onUnit" { 2 } else { 1 };
                let h = self.env.types.func(handler);
                match f.params.get_mut(at) {
                    Some(p) => p.ty = h,
                    None => return c,
                }
            }
            "events.send" => {
                f.params.truncate(1);
                f.params.extend(handler.params);
                f.rest = handler.rest;
            }
            _ => return c,
        }
        self.env.types.func(f)
    }

    /// Checks a call of `f`: arity (E2006, W2055, E2007), argument types
    /// (E2001), generic inference. Returns the results.
    fn call_func(
        &mut self,
        f: &Func,
        args: &Args,
        span: Span,
        expected: Option<TypeId>,
        report: bool,
    ) -> Option<Pack> {
        let exprs = Self::arg_exprs(args);
        let generic = !f.type_params.is_empty();
        // First pass: infer type arguments from the non-function arguments.
        let mut s = Subst::default();
        let mut arg_types: Vec<Option<TypeId>> = vec![None; exprs.len()];
        if generic {
            // Parameters that do not mention the type parameters type their
            // arguments contextually (literals keep their literal types).
            let mut probe = Subst::default();
            for p in &f.type_params {
                if self.env.params[p.0 as usize].pack {
                    probe.packs.insert(*p, Pack::single(Types::UNKNOWN));
                } else {
                    probe.types.insert(*p, Types::UNKNOWN);
                }
            }
            for (i, e) in exprs.iter().enumerate() {
                if matches!(e.kind, ExprKind::Function(_)) {
                    continue;
                }
                let fixed = self
                    .param_at(f, i)
                    .filter(|&p| subst::apply(&mut self.env.types, p, &probe) == p);
                let t = match fixed {
                    Some(p) if !is_multi(e) => self.check(e, Some(p)),
                    _ => self.infer(e),
                };
                arg_types[i] = Some(t);
                if let Some(p) = self.param_at(f, i) {
                    self.unify(p, t, &mut s);
                }
            }
            // Expected results give type arguments too.
            if let (Some(exp), Some(&r)) = (expected, f.ret.types.first()) {
                self.unify(r, exp, &mut s);
            }
            // The variadic tail binds a pack parameter. A function literal
            // (typed in the second pass) takes the type the pack was bound
            // to at its position by an earlier parameter, or `any`.
            if let Some(Rest::Pack(p)) = &f.rest {
                let fixed = f.params.len();
                let bound = s.packs.get(p).cloned();
                let mut pack = Pack::default();
                for (k, t) in arg_types.iter().skip(fixed).enumerate() {
                    let from_bound = || bound.as_ref().and_then(|b| b.types.get(k).copied());
                    pack.types.push(t.or_else(from_bound).unwrap_or(Types::ANY));
                }
                s.packs.insert(*p, pack);
            }
            // Unbound parameters are `any`.
            for p in &f.type_params {
                if self.env.params[p.0 as usize].pack {
                    s.packs.entry(*p).or_insert_with(Pack::any);
                } else {
                    s.types.entry(*p).or_insert(Types::ANY);
                }
            }
            // Inferred type arguments must satisfy their constraints (E2014).
            for p in &f.type_params {
                let info = &self.env.params[p.0 as usize];
                let (Some(c), Some(&t)) = (info.constraint, s.types.get(p)) else {
                    continue;
                };
                let name = info.name.clone();
                let c = subst::apply(&mut self.env.types, c, &s);
                if report && !self.env.assignable(t, c, self.mode) {
                    let msg = format!(
                        "`{}` does not satisfy the constraint `{}` of `{name}`",
                        self.env.display(t),
                        self.env.display(c)
                    );
                    self.report("E2014", span, msg);
                }
            }
        }
        let f = if generic {
            subst::apply_func(&mut self.env.types, f, &s)
        } else {
            f.clone()
        };
        // Second pass: check every argument against its parameter.
        let n_args = exprs.len();
        for (i, e) in exprs.iter().enumerate() {
            let p = self.param_at(&f, i);
            let multi = i + 1 == n_args && is_multi(e);
            let t = match arg_types[i] {
                Some(t) if !matches!(e.kind, ExprKind::Function(_)) => t,
                _ => {
                    if multi {
                        let pack = self.expr_inner(e, p);
                        // `...T` alone: its first value is a `T` (or none).
                        let first = match (pack.types.first(), &pack.rest) {
                            (Some(&t), _) => t,
                            (None, Some(Rest::Variadic(t))) => *t,
                            (None, _) => Types::NIL,
                        };
                        self.expr_types.insert((e.span.start, e.span.end), first);
                        // The extra values of a final multi-value argument.
                        for (j, &extra) in pack.types.iter().enumerate().skip(1) {
                            if let Some(pt) = self.param_at(&f, i + j)
                                && report
                            {
                                self.check_assignable(e.span, extra, pt);
                            }
                        }
                        first
                    } else {
                        self.check(e, p)
                    }
                }
            };
            match p {
                Some(pt) => {
                    if report {
                        self.check_assignable(e.span, t, pt);
                    }
                }
                None if report => {
                    let code = if f.unannotated && self.mode == super::query::Mode::Basic {
                        "W2055"
                    } else {
                        "E2006"
                    };
                    let msg = format!(
                        "too many arguments: expected {}, got {}",
                        f.params.len(),
                        n_args
                    );
                    self.report(code, e.span, msg);
                    break;
                }
                None => {}
            }
        }
        // Missing arguments (E2007): a final multi-value argument may supply them.
        let supplied = if exprs.last().is_some_and(|e| is_multi(e)) {
            usize::MAX
        } else {
            n_args
        };
        if report && supplied != usize::MAX {
            for p in f.params.iter().skip(supplied) {
                if !p.optional {
                    let name = p
                        .name
                        .as_deref()
                        .map(|n| format!(" `{n}`"))
                        .unwrap_or_default();
                    let msg = format!(
                        "missing argument{name} of type `{}`",
                        self.env.display(p.ty)
                    );
                    self.report("E2007", span, msg);
                    break;
                }
            }
        }
        Some(f.ret)
    }

    /// The parameter type at argument position `i`.
    fn param_at(&self, f: &Func, i: usize) -> Option<TypeId> {
        match f.params.get(i) {
            Some(p) => Some(p.ty),
            None => match &f.rest {
                Some(Rest::Variadic(t)) => Some(*t),
                Some(Rest::Pack(_)) => Some(Types::ANY),
                None => None,
            },
        }
    }

    /// Infers type arguments: binds parameters in `pattern` to the
    /// corresponding parts of `actual`.
    pub fn unify(&mut self, pattern: TypeId, actual: TypeId, s: &mut Subst) {
        match (
            self.env.types.get(pattern).clone(),
            self.env.types.get(actual).clone(),
        ) {
            (Ty::Param(p), _) => {
                let widened = self.env.types.widen(actual);
                match s.types.get(&p).copied() {
                    Some(existing) if existing != widened => {
                        let u = self.env.types.union(&[existing, widened]);
                        s.types.insert(p, u);
                    }
                    Some(_) => {}
                    None => {
                        s.types.insert(p, widened);
                    }
                }
            }
            (Ty::Array(a), Ty::Array(b)) => self.unify(a, b, s),
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.unify(k1, k2, s);
                self.unify(v1, v2, s);
            }
            (Ty::Map(k, v), Ty::Array(e)) => {
                self.unify(k, Types::NUMBER, s);
                self.unify(v, e, s);
            }
            (Ty::Map(k, v), Ty::Record(r)) => {
                if let Some((rk, rv)) = r.indexer {
                    self.unify(k, rk, s);
                    self.unify(v, rv, s);
                } else if !r.fields.is_empty() {
                    let keys: Vec<TypeId> =
                        r.fields.keys().map(|n| self.env.types.str(n)).collect();
                    let vals: Vec<TypeId> = r.fields.values().map(|f| f.ty).collect();
                    let ku = self.env.types.union(&keys);
                    let ku = self.env.types.widen(ku);
                    let vu = self.env.types.union(&vals);
                    self.unify(k, ku, s);
                    self.unify(v, vu, s);
                }
            }
            (Ty::Union(ps), _) => {
                // `T?` against `X?`: bind T to the non-nil part.
                let non_nil: Vec<TypeId> =
                    ps.iter().copied().filter(|&x| x != Types::NIL).collect();
                if non_nil.len() == 1 {
                    let a = self.env.types.non_nil(actual);
                    self.unify(non_nil[0], a, s);
                }
            }
            (Ty::Named(d1, a1), Ty::Named(d2, a2)) if d1 == d2 => {
                for (x, y) in a1.iter().zip(&a2) {
                    self.unify(*x, *y, s);
                }
            }
            (Ty::Func(pf), Ty::Func(af)) => {
                for (pp, ap) in pf.params.iter().zip(&af.params) {
                    self.unify(pp.ty, ap.ty, s);
                }
                for (pr, ar) in pf.ret.types.iter().zip(&af.ret.types) {
                    self.unify(*pr, *ar, s);
                }
                if let Some(Rest::Pack(p)) = &pf.ret.rest {
                    s.packs.insert(*p, af.ret.clone());
                }
            }
            (Ty::Record(pr), Ty::Record(ar)) => {
                for (n, f) in &pr.fields {
                    if let Some(af) = ar.fields.get(n) {
                        self.unify(f.ty, af.ty, s);
                    }
                }
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------- super --

    /// `super.name` / `super:name` inside a class body.
    fn super_member(&mut self, name: &str, span: Span) -> TypeId {
        let Some((def, is_static)) = self.classes.last().copied() else {
            return Types::ANY;
        };
        let bases = self.env.def(def).bases.clone();
        let Some(&base) = bases.first() else {
            self.report(
                "E3001",
                span,
                format!("class `{}` has no superclass", self.env.def(def).name),
            );
            return Types::ERROR;
        };
        if name.is_empty() {
            return base;
        }
        match self.env.types.get(base).clone() {
            Ty::Named(bd, args) => {
                if is_static {
                    self.env
                        .static_member(bd, name)
                        .map(|m| m.ty)
                        .unwrap_or(Types::ANY)
                } else {
                    match self.env.instance_member(bd, &args, name) {
                        Some((m, _)) => m.ty,
                        None => {
                            if !self.env.def(bd).native {
                                let msg =
                                    format!("`{}` has no member `{name}`", self.env.def(bd).name);
                                self.report("E2003", span, msg);
                            }
                            Types::ANY
                        }
                    }
                }
            }
            _ => Types::ANY,
        }
    }

    /// The API path of a field expression on a global (`C_X.Fn`).
    fn api_path_of(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Name(n) if self.lookup_var(&n.text).is_none() => Some(n.text.to_string()),
            ExprKind::Field { obj, name } => {
                self.api_path_of(obj).map(|p| format!("{p}.{}", name.text))
            }
            _ => None,
        }
    }

    /// `obj.name` is a method of a class or declared class.
    fn is_method_member(&mut self, obj: &Expr, name: &str) -> bool {
        let o = self.peek_type(obj);
        match self.env.types.get(o).clone() {
            Ty::Named(d, args) => self
                .env
                .instance_member(d, &args, name)
                .is_some_and(|(m, _)| m.kind == MemberKind::Method),
            _ => false,
        }
    }

    /// The type of `setmetatable(t, mt)` (spec §2.4.6): the fields of `t`,
    /// then those of `mt.__index` (methods included). Later additions to an
    /// open metatable are allowed through an indexer.
    fn with_metatable(&mut self, t: &Expr, mt: &Expr) -> TypeId {
        let tt = self.peek_type(t);
        let mtt = self.peek_type(mt);
        let index = self.field_type_quiet(mtt, "__index");
        let Ty::Record(mut r) = self.env.types.get(tt).clone() else {
            return tt;
        };
        if let Some(i) = index
            && let Ty::Record(ir) = self.env.types.get(i).clone()
        {
            for (k, f) in ir.fields {
                r.fields.entry(k).or_insert(f);
            }
            if ir.open || r.indexer.is_none() {
                r.indexer = Some((Types::STRING, Types::ANY));
            }
        } else if index.is_some() {
            r.indexer = Some((Types::STRING, Types::ANY));
        }
        r.open = false;
        self.env.types.record(r)
    }

    pub fn is_super(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Name(n) if n.text.as_ref() == "super")
            && !self.classes.is_empty()
            && self.lookup_var("super").is_none()
    }

    // ---------------------------------------------------------- iteration --

    /// The loop variable types of a generic `for` (spec §2.4.4).
    pub fn iteration_types(&mut self, values: &[TypeId], span: Span) -> Vec<TypeId> {
        let Some(&iter) = values.first() else {
            return Vec::new();
        };
        match self.env.types.get(iter).clone() {
            Ty::Func(f) => {
                // A generic iterator (`next`): its parameters are `any`.
                let f = self.instantiate_any(&f);
                let mut out: Vec<TypeId> = f.ret.types.clone();
                if let Some(Rest::Variadic(t)) = f.ret.rest {
                    out.push(t);
                }
                if let Some(first) = out.first_mut() {
                    *first = self.env.types.non_nil(*first);
                }
                out
            }
            Ty::Any
            | Ty::Error
            | Ty::Function
            | Ty::Intersection(_)
            | Ty::Table
            | Ty::Named(..) => Vec::new(),
            Ty::Nil => {
                self.report("E2004", span, "the iterator of this `for` is `nil`");
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// Expressions that may produce several values.
pub fn is_multi(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Call { .. }
            | ExprKind::Method { .. }
            | ExprKind::OptCall { .. }
            | ExprKind::OptMethod { .. }
            | ExprKind::Vararg
    )
}
