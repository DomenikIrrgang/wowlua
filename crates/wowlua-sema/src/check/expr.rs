//! Expressions (spec chapter 2): inference and contextual checking, names
//! and globals, tables, fields, operators and calls (arity, generics,
//! overloads).

use super::body::{Checker, VarId};
use super::env::{DefKind, FnSig, MemberKind, Visibility};
use super::subst::{self, Subst};
use super::types::*;
use super::types::{Field, Func, Types};
use rustc_hash::FxHashSet;
use wowlua_span::Span;
use wowlua_syntax::view::*;

/// `wowlua:test`'s own matchers (and `never`): other names on an
/// `expect(…)` are custom matchers.
const BUILTIN_MATCHERS: &[&str] = &[
    "toBe",
    "toEqual",
    "toBeNil",
    "toBeTruthy",
    "toBeFalsy",
    "toBeSecret",
    "toBeInstanceOf",
    "toContain",
    "toHaveLength",
    "toBeCloseTo",
    "toThrow",
    "toHaveBeenCalled",
    "toHaveBeenCalledTimes",
    "toHaveBeenCalledWith",
    "toBeShown",
    "toBeVisible",
    "toHaveText",
    "toHaveSize",
    "toBeAnchoredTo",
    "toHaveLines",
    "toMatchSnapshot",
    "toBeBlocked",
    "toBeSecure",
    "never",
];

/// What an expression gives: one value (without a heap allocation), or a
/// pack (calls, `...`).
pub(crate) enum Values {
    One(TypeId),
    Many(Pack),
}

impl Values {
    pub(crate) fn into_pack(self) -> Pack {
        match self {
            Values::One(t) => Pack::single(t),
            Values::Many(p) => p,
        }
    }
}

impl From<Pack> for Values {
    fn from(p: Pack) -> Values {
        Values::Many(p)
    }
}

impl Checker<'_, '_> {
    /// The type of an expression.
    pub fn infer(&mut self, e: Expr<'_>) -> TypeId {
        self.check(e, None)
    }

    /// The type of an expression where `expected` is wanted (contextual
    /// typing, spec §2.4.3). Mismatches are reported by the caller.
    pub fn check(&mut self, e: Expr<'_>, expected: Option<TypeId>) -> TypeId {
        let before = self.diagnostics.len();
        let values = stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_inner(e, expected));
        self.explain_closure_nil(e, before);
        let t = match values {
            Values::One(t) => t,
            Values::Many(pack) => pack
                .types
                .first()
                .copied()
                .unwrap_or_else(|| match &pack.rest {
                    Some(Rest::Variadic(t)) => self.env.types.optional(*t),
                    _ => Types::NIL,
                }),
        };
        self.expr_types.insert((e.span().start, e.span().end), t);
        t
    }

    /// All values of a multi-value expression (calls, `...`).
    pub fn infer_multi(&mut self, e: Expr<'_>) -> Pack {
        let before = self.diagnostics.len();
        let pack =
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_inner(e, None)).into_pack();
        self.explain_closure_nil(e, before);
        let first = pack.types.first().copied().unwrap_or(Types::NIL);
        self.expr_types
            .insert((e.span().start, e.span().end), first);
        pack
    }

    /// E2005 on a local of an enclosing function that is assigned after
    /// its declaration (`opts = opts or {}`): closures see such a local as
    /// declared (spec §2.5), which the help says, with the ways around it.
    fn explain_closure_nil(&mut self, e: Expr<'_>, before: usize) {
        if self.diagnostics.len() == before {
            return;
        }
        let (obj, at) = match e.kind() {
            ExprKind::Field { obj, name } | ExprKind::Method { obj, name, .. } => {
                (obj, name.span())
            }
            ExprKind::Index { obj, .. } => (obj, e.span()),
            ExprKind::Call { callee, .. } => (callee, e.span()),
            ExprKind::Unary {
                op: UnOp::Len,
                operand,
            } => (operand, operand.span()),
            _ => return,
        };
        let mut obj = obj;
        while let ExprKind::Paren(inner) = obj.kind() {
            obj = inner;
        }
        let ExprKind::Name(n) = obj.kind() else {
            return;
        };
        let Some(v) = self.lookup_var(n.text()) else {
            return;
        };
        let var = &self.vars[v.0 as usize];
        if var.depth >= self.depth || var.never_reassigned {
            return;
        }
        let name = n.text();
        let help = format!(
            "`{name}` is assigned after its declaration, so closures see it as `{}`, not as narrowed outside (spec §2.5); if it is a parameter, give it a default value (`{name}: T = …`), or copy the checked value into a new local for the closure",
            self.env.display(var.base)
        );
        for d in &mut self.diagnostics[before..] {
            if d.code == Some("E2005")
                && d.help.is_none()
                && d.labels.first().is_some_and(|l| l.span == at)
            {
                d.help = Some(help.clone());
            }
        }
    }

    /// The type an expression was checked with (for predicates).
    pub fn peek_type(&self, e: Expr<'_>) -> TypeId {
        self.expr_types
            .get(&(e.span().start, e.span().end))
            .copied()
            .unwrap_or(Types::ANY)
    }

    /// A list of expressions as values (the last one expands).
    pub fn expr_list(&mut self, es: Exprs<'_>, expected: &[Option<TypeId>]) -> Vec<TypeId> {
        self.expr_list_into(es, expected, &[])
    }

    /// [`Self::expr_list`] for the names it is spread over (a `local` or an
    /// assignment): a name past what the list's last call can give is
    /// always `nil` (W2062).
    pub fn expr_list_into(
        &mut self,
        es: Exprs<'_>,
        expected: &[Option<TypeId>],
        names: &[(Span, String)],
    ) -> Vec<TypeId> {
        let pack = self.expr_pack(es, expected);
        if let Some(last) = es.iter().next_back()
            && let Some((callee, key)) = self.cap_key(last)
            && names.len() > es.len() - 1
        {
            let use_ = CapUse::Names {
                before: es.len() - 1,
                names: names.to_vec(),
            };
            match self.cap_of(&key) {
                Some(gives) => self.report_past(&callee, gives, &use_),
                // (a method defined further down: judged at the file's end)
                None => self.pending_caps.push((key, callee, use_)),
            }
        }
        let mut out = pack.types.clone();
        if let Some(Rest::Variadic(t)) = pack.rest {
            while out.len() < expected.len().max(es.len()) {
                let o = self.env.types.optional(t);
                out.push(o);
            }
        }
        out
    }

    /// `select(n, ..., f(...))` with a literal `n` past what the values
    /// can be, `f` a `local function` of known count: always `nil` (W2062).
    fn select_past_results(&mut self, callee: Expr<'_>, args: Args<'_>) {
        let ExprKind::Name(n) = callee.kind() else {
            return;
        };
        if n.text() != "select" || self.lookup_var("select").is_some() {
            return;
        }
        let exprs = args.exprs();
        if exprs.len() < 2 {
            return;
        }
        let (Some(first), Some(last)) = (exprs.iter().next(), exprs.iter().next_back()) else {
            return;
        };
        let ExprKind::Number(lit) = first.kind() else {
            return;
        };
        let Some(index) = lit.value() else {
            return;
        };
        let Some((name, key)) = self.cap_key(last) else {
            return;
        };
        if index.fract() != 0.0 || index < 1.0 {
            return;
        }
        let use_ = CapUse::Select {
            before: exprs.len() - 2,
            index,
            span: first.span(),
        };
        match self.cap_of(&key) {
            Some(gives) => self.report_past(&name, gives, &use_),
            None => self.pending_caps.push((key, name, use_)),
        }
    }

    /// W2062 for a use of `gives` values (see [`CapUse`]).
    pub(crate) fn report_past(&mut self, callee: &str, gives: usize, use_: &CapUse) {
        let s = if gives == 1 { "" } else { "s" };
        match use_ {
            CapUse::Names { before, names } => {
                let total = before + gives;
                // a name starting with `_` takes a value it doesn't want (as
                // for W1003): the first other one past the count
                if let Some((i, (span, name))) = names
                    .iter()
                    .enumerate()
                    .skip(total)
                    .find(|(_, (_, n))| !n.starts_with('_'))
                {
                    let more = if names.len() > i + 1 {
                        " (and the names after it)"
                    } else {
                        ""
                    };
                    self.report(
                        "W2062",
                        *span,
                        format!("`{callee}` gives at most {gives} value{s}: `{name}`{more} is always nil"),
                    );
                }
            }
            CapUse::Select {
                before,
                index,
                span,
            } => {
                if *index as usize > before + gives {
                    self.report(
                        "W2062",
                        *span,
                        format!("`{callee}` gives at most {gives} value{s}: `select({index})` is always nil"),
                    );
                }
            }
        }
    }

    /// A call of a function of this file (a `local function`, a module's
    /// `function f()`, an imported one) or a method of a file-level table
    /// (`M:f()`, `M.f()`, `self:f()` in `M`'s methods) whose `return`s
    /// each give a fixed number of values: its name and the most it gives
    /// (W2062).
    fn cap_key(&self, e: Expr<'_>) -> Option<(String, CapKey)> {
        match e.kind() {
            ExprKind::Call { callee, .. } | ExprKind::OptCall { callee, .. } => match callee.kind()
            {
                ExprKind::Name(n) => {
                    let var = self.lookup_var(n.text())?;
                    Some((n.text().to_string(), CapKey::Var(var)))
                }
                ExprKind::Field { obj, name } => {
                    let table = self.cap_table(obj)?;
                    Some((
                        format!("{}.{}", obj_text(obj), name.text()),
                        CapKey::Method(table, name.text().into()),
                    ))
                }
                _ => None,
            },
            ExprKind::Method { obj, name, .. } | ExprKind::OptMethod { obj, name, .. } => {
                let table = self.cap_table(obj)?;
                Some((
                    format!("{}:{}", obj_text(obj), name.text()),
                    CapKey::Method(table, name.text().into()),
                ))
            }
            _ => None,
        }
    }

    /// The count a [`CapKey`] has now, if any.
    pub(crate) fn cap_of(&self, key: &CapKey) -> Option<usize> {
        match key {
            CapKey::Var(v) => self.result_caps.get(v).copied(),
            CapKey::Method(t, m) => self.method_caps.get(&(*t, m.clone())).copied(),
        }
    }

    /// The file-level table a method is looked up on: a variable that holds
    /// one, or a method's `self` (the table it was defined on).
    pub(crate) fn cap_table(&self, obj: Expr<'_>) -> Option<VarId> {
        let ExprKind::Name(n) = obj.kind() else {
            return None;
        };
        let var = self.lookup_var(n.text())?;
        Some(self.self_owners.get(&var).copied().unwrap_or(var))
    }

    pub fn expr_pack(&mut self, es: Exprs<'_>, expected: &[Option<TypeId>]) -> Pack {
        let mut types = Vec::new();
        let mut rest = None;
        for (i, e) in es.iter().enumerate() {
            let want = expected.get(i).copied().flatten();
            if i + 1 == es.len() && is_multi(e) {
                let mut p = self.expr_inner(e, want).into_pack();
                if let (Some(w), Some(&first)) = (want, p.types.first()) {
                    let _ = (w, first);
                }
                let first = p.types.first().copied().unwrap_or(Types::NIL);
                self.expr_types
                    .insert((e.span().start, e.span().end), first);
                types.append(&mut p.types);
                rest = p.rest;
            } else {
                types.push(self.check(e, want));
            }
        }
        Pack { types, rest }
    }

    fn expr_inner(&mut self, e: Expr<'_>, expected: Option<TypeId>) -> Values {
        let single = Values::One;
        match e.kind() {
            ExprKind::Nil => single(Types::NIL),
            ExprKind::True => single(Types::TRUE),
            ExprKind::False => single(Types::FALSE),
            ExprKind::Number(n) => single(match n.value() {
                Some(v) => self.env.types.num(v),
                None => Types::NUMBER,
            }),
            ExprKind::String(s) => single(self.env.types.str(&String::from_utf8_lossy(&s.value()))),
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.infer(x);
                    }
                }
                single(Types::STRING)
            }
            ExprKind::Vararg => self.vararg_pack().into(),
            ExprKind::Function(f) => {
                let exp = expected.and_then(|t| self.contextual_func(t));
                // A function passed as an argument or with an expected type
                // is a callback.
                self.callback_next = expected.is_some() || self.callbacks.contains(&e.span());
                single(self.function_body(&FnSig::of(f), exp, None))
            }
            ExprKind::Table(fields) => single(self.table(fields, expected)),
            // An existence test of an undeclared global: `nil`; inside the
            // region the test guards: `any`.
            ExprKind::Name(n)
                if self.lookup_var(n.text()).is_none()
                    && !self.env.declared.has_global(n.text())
                    && !self.dynamic_env
                    && n.text() != "_G"
                    && !self.extra_globals.contains(n.text())
                    && !self.assigned_globals.contains(n.text())
                    && (self.probes.contains(&e.span())
                        || self.guard.present.contains(n.text())) =>
            {
                single(if self.probes.contains(&e.span()) {
                    Types::NIL
                } else {
                    Types::ANY
                })
            }
            ExprKind::Name(n) if self.probes.contains(&e.span()) => {
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
                    self.value_checks.insert(e.span(), c);
                }
                // Assertions between incomparable types (E2015).
                if !self.castable(from, to) {
                    let mut msg = format!(
                        "cannot assert `{}` as `{}`",
                        self.env.display(from),
                        self.env.display(to)
                    );
                    // Which members of a union keep each direction from
                    // holding.
                    let mut why = Vec::new();
                    for (a, b) in [(from, to), (to, from)] {
                        let members = self.env.types.members(a);
                        if members.len() < 2 {
                            continue;
                        }
                        if let Some(&m) = members
                            .iter()
                            .find(|&&m| !self.env.assignable(m, b, self.mode))
                        {
                            why.push(format!(
                                "`{}` does not fit `{}`",
                                self.env.display(m),
                                self.env.display(b)
                            ));
                        }
                    }
                    if !why.is_empty() {
                        msg.push_str(&format!(
                            " (neither type fits the other: {})",
                            why.join(", and ")
                        ));
                    }
                    self.report("E2015", e.span(), msg);
                    let help = self.cast_help(from, to);
                    if let Some(d) = self.diagnostics.last_mut() {
                        d.help = Some(help);
                    }
                }
                single(to)
            }
            ExprKind::Field { obj, name } | ExprKind::OptField { obj, name } => {
                if self.is_super(obj) {
                    return single(self.super_member(name.text(), name.span()));
                }
                let probed = self.probes.contains(&e.span());
                // Only shared mode and existence guards read the path.
                let path = (self.shared.is_some() || !self.guard.present.is_empty())
                    .then(|| self.api_path_of(e))
                    .flatten();
                if self.shared.is_some()
                    && !probed
                    && let Some(path) = &path
                {
                    self.check_available(path, name.span());
                }
                let guarded = path
                    .as_ref()
                    .is_some_and(|p| self.guard.present.contains(p));
                let (o, chained) = self.chain_obj(obj);
                let opt = matches!(e.kind(), ExprKind::OptField { .. });
                let o = if opt { self.env.types.non_nil(o) } else { o };
                if let Some(t) = self.custom_matcher(obj, o, name) {
                    return single(t);
                }
                // A member the type lacks, in an existence test (`nil`) or
                // in the region such a test guards (`any`).
                let missing = (probed || guarded)
                    && self.member_type(o, name.text(), name.span(), true) == Some(Types::ERROR);
                let t = if missing {
                    if probed { Types::NIL } else { Types::ANY }
                } else {
                    self.member_type(o, name.text(), name.span(), false)
                        .unwrap_or(Types::ANY)
                };
                let t = if opt || chained {
                    self.chain_end(e, t)
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
                let (o, chained) = self.chain_obj(obj);
                let opt = matches!(e.kind(), ExprKind::OptIndex { .. });
                let o = if opt { self.env.types.non_nil(o) } else { o };
                let k = self.infer(key);
                let t = self.index_type(o, k, key, e.span());
                single(if opt || chained {
                    self.chain_end(e, t)
                } else {
                    t
                })
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                self.mark_callbacks(args);
                self.select_past_results(callee, args);
                if let ExprKind::Name(n) = callee.kind()
                    && n.text() == "super"
                    && !self.classes.is_empty()
                {
                    self.args_types(args, &[]);
                    return Values::Many(Pack::empty());
                }
                let (c, chained) = self.chain_obj(callee);
                let opt = matches!(e.kind(), ExprKind::OptCall { .. });
                let c = if opt { self.env.types.non_nil(c) } else { c };
                // A method called with `.` and no receiver (E2008).
                if args.exprs().is_empty()
                    && let ExprKind::Field { obj, name } = callee.kind()
                    && self.is_method_member(obj, name.text())
                {
                    self.report(
                        "E2008",
                        e.span(),
                        format!("`{}` is a method; call it with `:`", name.text()),
                    );
                    return Values::Many(Pack::any());
                }
                self.check_yield_boundary(callee, args);
                let c = self.stdlib_call_type(callee, args, c);
                let mut results = self.call(c, None, args, e.span(), expected);
                self.forget_passed(None, args);
                self.stdlib_after_call(callee, args);
                // `assert(v)` returns `v` only where it is truthy.
                if let ExprKind::Name(n) = callee.kind()
                    && n.text() == "assert"
                    && self.lookup_var("assert").is_none()
                    && let Some(first) = results.types.first_mut()
                {
                    *first = self.truthy(*first);
                }
                if let Some(t) = self.saved_bind_type(callee, args) {
                    return Values::One(t);
                }
                if self.secret_locale_call(callee, args) {
                    let secret = self.env.types.intern(Ty::Secret(Types::STRING));
                    return Values::One(self.env.types.union(&[Types::STRING, secret]));
                }
                // `setmetatable(t, mt)`: `t` with `mt.__index` as fallback.
                if let ExprKind::Name(n) = callee.kind()
                    && n.text() == "setmetatable"
                    && self.lookup_var("setmetatable").is_none()
                    && let Some([t, mt]) = args.exprs().exactly()
                {
                    return Values::One(self.with_metatable(t, mt));
                }
                // The end of an optional chain: one value, `nil` when it
                // short-circuits.
                if opt || chained {
                    let first = results.types.first().copied().unwrap_or(Types::NIL);
                    return single(self.chain_end(e, first));
                }
                results.into()
            }
            ExprKind::Method { obj, name, args } | ExprKind::OptMethod { obj, name, args } => {
                self.mark_callbacks(args);
                if self.is_super(obj) {
                    let m = self.super_member(name.text(), name.span());
                    let instance = self.scope.self_ty.unwrap_or(Types::ANY);
                    return self
                        .call(m, Some(instance), args, e.span(), expected)
                        .into();
                }
                let (o, chained) = self.chain_obj(obj);
                let opt = matches!(e.kind(), ExprKind::OptMethod { .. });
                let o = if opt { self.env.types.non_nil(o) } else { o };
                // A function field a condition narrowed (`if t.f then t:f() end`).
                let narrowed = self
                    .field_path(obj)
                    .map(|(r, p)| (r, format!("{p}.{}", name.text())))
                    .or_else(|| match obj.kind() {
                        ExprKind::Name(n) => match self.lookup_var(n.text()) {
                            Some(r) => Some((r, name.text().to_string())),
                            None => self.env.declared.has_global(n.text()).then(|| {
                                (
                                    super::narrow::GLOBAL_ROOT,
                                    format!("{}.{}", n.text(), name.text()),
                                )
                            }),
                        },
                        _ => None,
                    })
                    .and_then(|key| self.field_vars.get(&key.0)?.get(key.1.as_str()).copied())
                    .and_then(|v| self.flow.get(&v).copied());
                let m = match narrowed {
                    Some(t) => self.drop_first(t),
                    None => self.method_type(o, name.text(), name.span()),
                };
                let m = self.registry_method_type(o, name.text(), args, m);
                self.check_router_add(o, name.text(), args);
                let results = self.call(m, Some(o), args, e.span(), expected);
                self.forget_passed(Some(obj), args);
                if opt || chained {
                    let first = results.types.first().copied().unwrap_or(Types::NIL);
                    return single(self.chain_end(e, first));
                }
                results.into()
            }
            ExprKind::Binary { op, lhs, rhs } => {
                single(self.binary(op, lhs, rhs, e.span(), expected))
            }
            ExprKind::Unary { op, operand } => {
                if op == UnOp::Not {
                    self.mark_probes(operand);
                }
                let t = self.infer(operand);
                single(match op {
                    UnOp::Not => Types::BOOLEAN,
                    UnOp::Len => {
                        self.check_secret(t, operand.span());
                        if self.mode == super::query::Mode::Strict
                            && t != Types::ANY
                            && self.env.types.is_optional(t)
                        {
                            let msg = self.nil_message(t);
                            self.report("E2005", operand.span(), msg);
                        }
                        Types::NUMBER
                    }
                    UnOp::Neg | UnOp::BNot => {
                        self.check_secret(t, operand.span());
                        self.arith_operand(t, operand.span());
                        Types::NUMBER
                    }
                })
            }
            ExprKind::Error => single(Types::ERROR),
        }
    }

    /// E2015's help: how to get the asserted type (an assertion needs one
    /// type assignable to the other, spec §1.4.3).
    fn cast_help(&mut self, from: TypeId, to: TypeId) -> String {
        let to_text = self.env.display(to);
        if self.is_secret(from) && !self.is_secret(to) {
            format!(
                "narrow a possibly secret value with `canaccessvalue` or `issecretvalue` before asserting it, or force the type with `:: any :: {to_text}`"
            )
        } else {
            format!("to force the type, write `:: any :: {to_text}`")
        }
    }

    /// `a :: b` is allowed: one fits the other or both are tables, with
    /// `a` as it is or with its `nil` left out (`TabButton[]? :: Button[]`,
    /// decision L39).
    fn castable(&mut self, a: TypeId, b: TypeId) -> bool {
        let direct = |c: &mut Self, x: TypeId, y: TypeId| {
            c.env.assignable(x, y, c.mode) || c.env.assignable(y, x, c.mode) || c.comparable(x, y)
        };
        if direct(self, a, b) {
            return true;
        }
        let x = self.env.types.non_nil(a);
        x != a && direct(self, x, b)
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
                    | Ty::ClassOfParam(_)
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
                Ty::Func(f) => return Some(std::sync::Arc::unwrap_or_clone(f)),
                Ty::Intersection(ms) => {
                    for x in ms {
                        if let Ty::Func(f) = self.env.types.get(x).clone() {
                            return Some(std::sync::Arc::unwrap_or_clone(f));
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
    fn mark_callbacks(&mut self, args: Args<'_>) {
        for a in args.exprs() {
            let mut a = a;
            while let ExprKind::Paren(inner) = a.kind() {
                a = inner;
            }
            if matches!(a.kind(), ExprKind::Function(_)) {
                self.callbacks.insert(a.span());
            }
        }
    }

    /// The type of a name: a local, an import, or a global.
    pub fn name_type(&mut self, n: Name<'_>) -> TypeId {
        if let Some(v) = self.lookup_var(n.text()) {
            // A parameter read in its own default or an earlier one's,
            // before it is set (a function literal there runs later).
            if self.unset_params.contains(&v) && self.vars[v.0 as usize].depth == self.depth {
                self.report(
                    "E1004",
                    n.span(),
                    format!(
                        "`{}` here is the parameter, which is not set yet (a default reads only earlier parameters); to read an outer `{}`, give the parameter another name",
                        n.text(),
                        n.text()
                    ),
                );
            }
            return self.var_type(v);
        }
        self.check_available(n.text(), n.span());
        if n.text() == "super" && !self.classes.is_empty() {
            return self.super_member("", n.span());
        }
        match self.env.declared.global(n.text()) {
            Some(t) => t,
            None if n.text() == "_G" => Types::TABLE,
            None if self.dynamic_env
                || self.extra_globals.contains(n.text())
                || self.assigned_globals.contains(n.text()) =>
            {
                Types::ANY
            }
            None => {
                let mut msg = format!("`{}` is not a declared global", n.text());
                if n.text() == "os" {
                    // Code written for plain Lua (decision S37).
                    msg.push_str(": WoW has no `os` library; use the globals `time()`, `date(…)` and `difftime(…)`, and `debugprofilestop()` (milliseconds) for `os.clock()`");
                }
                let code = match self.mode {
                    super::query::Mode::Strict => "E1001",
                    super::query::Mode::Basic => "W1001",
                };
                self.report(code, n.span(), msg);
                Types::ANY
            }
        }
    }

    /// Assigning a global (W1002 / E1002 unless declared).
    pub fn assign_global(&mut self, n: Name<'_>, value: TypeId) {
        self.forget_fields(super::narrow::GLOBAL_ROOT, Some(n.text()));
        match self.env.declared.global(n.text()) {
            Some(t) => self.check_assignable(n.span(), value, t),
            None if self.dynamic_env || self.extra_globals.contains(n.text()) => {}
            None => {
                let code = match self.mode {
                    super::query::Mode::Strict => "E1002",
                    super::query::Mode::Basic => "W1002",
                };
                self.report(
                    code,
                    n.span(),
                    format!("assigns the undeclared global `{}` (declare it, use `_G.{}`, or make it `local`)", n.text(), n.text()),
                );
            }
        }
    }

    /// The type an assignment target expects (for contextual typing).
    pub fn target_type(&mut self, t: Expr<'_>) -> Option<TypeId> {
        match t.kind() {
            ExprKind::Name(n) => match self.lookup_var(n.text()) {
                Some(v) => self.vars[v.0 as usize].declared,
                None => self.env.declared.global(n.text()),
            },
            ExprKind::Field { obj, name } => {
                let o = self.peek_or_infer(obj);
                self.field_type_quiet(o, name.text())
            }
            // `list[i] = …`, `map[k] = …`: the element type.
            ExprKind::Index { obj, .. } => {
                let o = self.peek_or_infer(obj);
                let o = self.env.types.non_nil(o);
                match self.env.types.get(o).clone() {
                    Ty::Array(element) => Some(element),
                    Ty::Map(_, value) => Some(value),
                    Ty::Record(r) => r.indexer.map(|(_, value)| value),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn peek_or_infer(&mut self, e: Expr<'_>) -> TypeId {
        match e.kind() {
            ExprKind::Name(n) => match self.lookup_var(n.text()) {
                Some(v) => self.var_type(v),
                None => self.env.declared.global(n.text()).unwrap_or(Types::ANY),
            },
            _ => self.infer(e),
        }
    }

    /// A field's declared type, without diagnostics.
    /// The object of a chain link (`obj` in `obj.f`, `obj[k]`, `obj:m()`,
    /// `obj()`): its type, and whether it is part of an optional chain,
    /// which continues from its type before the short-circuit's `nil`
    /// (spec §1.4.1).
    fn chain_obj(&mut self, obj: Expr<'_>) -> (TypeId, bool) {
        let t = self.infer(obj);
        match self.chain_inner.get(&(obj.span().start, obj.span().end)) {
            Some(&inner) => (inner, true),
            None => (t, false),
        }
    }

    /// A link of an optional chain: its type is `t`, or `nil` when the chain
    /// short-circuits.
    fn chain_end(&mut self, e: Expr<'_>, t: TypeId) -> TypeId {
        self.chain_inner.insert((e.span().start, e.span().end), t);
        self.env.types.optional(t)
    }

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
                .or_else(|| self.env.view(d).member(name).map(|m| m.ty)),
            _ => None,
        }
    }

    pub fn assign_target(&mut self, t: Expr<'_>, value: TypeId) {
        match t.kind() {
            ExprKind::Name(n) => match self.lookup_var(n.text()) {
                Some(v) => {
                    if self.vars[v.0 as usize].is_const {
                        self.report(
                            "E2010",
                            n.span(),
                            format!("cannot assign to `{}`", n.text()),
                        );
                    }
                    self.assign_var(v, value, n.span())
                }
                None => self.assign_global(n, value),
            },
            ExprKind::Field { obj, name } => {
                let o = self.infer(obj);
                let declared = self.declared_of(obj);
                self.assign_field(o, name.text(), name.span(), value, declared);
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
                if let ExprKind::String(s) = key.kind() {
                    let name = String::from_utf8_lossy(&s.value()).into_owned();
                    let declared = self.declared_of(obj);
                    self.assign_field(o, &name, key.span(), value, declared);
                    return;
                }
                match self.env.types.get(o).clone() {
                    Ty::Array(e) => {
                        self.check_assignable(key.span(), k, Types::NUMBER);
                        self.check_assignable(t.span(), value, e);
                    }
                    Ty::Map(kt, vt) => {
                        self.check_assignable(key.span(), k, kt);
                        let opt = self.env.types.optional(vt);
                        self.check_assignable(t.span(), value, opt);
                    }
                    _ => {
                        let members = self.env.types.members(o);
                        if let Some(&m) = members.iter().find(|&&m| self.unindexable(m)) {
                            let msg =
                                format!("cannot index a value of type `{}`", self.env.display(m));
                            self.report("E2003", key.span(), msg);
                        }
                    }
                }
            }
            _ => {
                self.infer(t);
            }
        }
    }

    /// The annotated type of the variable `e` names, if it is one.
    fn declared_of(&self, e: Expr<'_>) -> Option<TypeId> {
        match e.kind() {
            ExprKind::Name(n) => self
                .lookup_var(n.text())
                .and_then(|v| self.vars[v.0 as usize].declared),
            _ => None,
        }
    }

    /// `o.name = value`; `declared` is the annotated type of the variable
    /// `o` is read from, which the assignment checks against: an annotated
    /// `const` table takes the fields its type declares or its indexer
    /// allows, though its value is a sealed literal.
    fn assign_field(
        &mut self,
        o: TypeId,
        name: &str,
        span: Span,
        value: TypeId,
        declared: Option<TypeId>,
    ) {
        // An unannotated `const` is "declared" as its literal: an open one
        // is the table it started as, which `o` extends.
        if let Some(d) = declared
            && d != o
            && !matches!(self.env.types.get(d), Ty::Record(r) if r.open)
        {
            // Where the variable is narrowed (`if item then` on an
            // `Item | false`), the declared members its value fits.
            let fitting: Vec<TypeId> = self
                .env
                .types
                .members(d)
                .into_iter()
                .filter(|&dm| {
                    self.env
                        .types
                        .members(o)
                        .into_iter()
                        .any(|om| self.env.assignable(om, dm, super::query::Mode::Strict))
                })
                .collect();
            let d = if fitting.is_empty() {
                d
            } else {
                self.env.types.union(&fitting)
            };
            return self.assign_field(d, name, span, value, None);
        }
        let members = self.env.types.members(o);
        for m in members {
            match self.env.types.get(m).clone() {
                // A field read through `__index`: the table's own replaces it.
                Ty::Record(_)
                    if self
                        .fallback_fields
                        .get(&m)
                        .is_some_and(|f| f.contains(name)) =>
                {
                    self.extend_open(m, name, value)
                }
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
                    Some((mem, owner)) => {
                        // A readonly field: only in its own class's
                        // constructor (spec §3.1.1).
                        let in_own_ctor = self.ctor_of == Some(owner);
                        if (mem.kind == MemberKind::Getter && mem.readonly)
                            || (mem.readonly && mem.kind == MemberKind::Field && !in_own_ctor)
                        {
                            self.report("E2010", span, format!("`{name}` is read-only"));
                        }
                        // A method's stored value takes `self` first.
                        let want = if mem.kind == MemberKind::Method {
                            self.env.with_self(mem.ty, m)
                        } else {
                            mem.ty
                        };
                        self.check_assignable(span, value, want);
                    }
                    None => {
                        if self.env.indexer_of(d).is_none()
                            && self.env.view(d).kind() != DefKind::Enum
                        {
                            if name == "__wowlua" {
                                self.report(
                                    "E2003",
                                    span,
                                    "`__wowlua` is reserved for the wowlua runtime",
                                );
                            } else if !self.env.view(d).native()
                                || self.env.view(d).widget().is_none()
                            {
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
                    if self.env.view(d).kind() == DefKind::Enum {
                        self.report(
                            "E2010",
                            span,
                            format!("cannot assign to the enum member `{name}`"),
                        );
                    } else if let Some(mem) = self.env.static_member(d, name) {
                        self.check_assignable(span, value, mem.ty);
                    }
                }
                _ if self.unindexable(m) => {
                    let msg = format!("cannot index a value of type `{}`", self.env.display(m));
                    self.report("E2003", span, msg);
                }
                _ => {}
            }
        }
    }

    /// A value Lua cannot index (functions, numbers, booleans): reading or
    /// writing one of its fields is an error at run time.
    pub(crate) fn unindexable(&self, t: TypeId) -> bool {
        matches!(
            self.env.types.get(t),
            Ty::Func(_) | Ty::Function | Ty::Number | Ty::Num(_) | Ty::Boolean | Ty::Bool(_)
        )
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
                if matches!(op, CompoundOp::Div | CompoundOp::IDiv | CompoundOp::Mod) {
                    self.zero_divisor(value, span);
                }
                Types::NUMBER
            }
        }
    }

    // ----------------------------------------------------------- tables --

    fn table(&mut self, fields: TableFields<'_>, expected: Option<TypeId>) -> TypeId {
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
            // One table member (`string | string[]`): the literal is that one.
            if tables.len() == 1 {
                return self.table(fields, Some(tables[0]));
            }
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
        // An expected record's indexer for numeric keys types the list part
        // (`{ [number]: V, n?: number }`, spec §2.2.4).
        let list_of_record = match &exp_ty {
            Some(Ty::Record(r)) => r.indexer.and_then(|(k, v)| {
                self.env
                    .assignable(Types::NUMBER, k, super::query::Mode::Basic)
                    .then_some(v)
            }),
            _ => None,
        };
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
                        exp.and_then(|t| self.field_type_quiet(t, name.text()))
                            .or(match &exp_ty {
                                Some(Ty::Map(_, v)) => Some(*v),
                                _ => None,
                            });
                    // Strict mode: a literal's unknown field for a record
                    // type is a typo (spec §2.2.4, E2003).
                    if self.mode == super::query::Mode::Strict
                        && let Some(Ty::Record(r)) = &exp_ty
                        && r.indexer.is_none()
                        && !r.fields.contains_key(name.text())
                    {
                        let msg = format!(
                            "`{}` is not a field of `{}`",
                            name.text(),
                            self.env.display(exp.expect("expected"))
                        );
                        self.report("E2003", name.span(), msg);
                    }
                    let t = self.check(value, want);
                    if let Some(w) = want {
                        self.check_assignable(value.span(), t, w);
                    }
                    record.fields.insert(
                        name.text().into(),
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
                    if let ExprKind::String(s) = key.kind() {
                        record.fields.insert(
                            String::from_utf8_lossy(&s.value()).into(),
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
                        _ => list_of_record,
                    };
                    if i + 1 == fields.len() && is_multi(value) {
                        let p = self.expr_inner(value, want).into_pack();
                        for t in p.types {
                            array.push(t);
                        }
                        if let Some(Rest::Variadic(t)) = p.rest {
                            array.push(t);
                        }
                    } else {
                        let t = self.check(value, want);
                        // A record's indexer is not compared as a whole: each
                        // element is checked, as a named field is.
                        if let Some(w) = list_of_record {
                            self.check_assignable(value.span(), t, w);
                        }
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
            _ => list_of_record,
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
            // A list for a record with a numeric indexer is that record
            // without its (optional) named fields, not an array.
            (Some(e), true, None) if list_of_record.is_none() => self.env.types.array(e),
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
                    if self.env.view(d).kind() == DefKind::Enum {
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
                Ty::Never | Ty::Param(_) | Ty::ClassOfParam(_) | Ty::EventArgs(_) => {
                    Some(Types::ANY)
                }
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
        match self.env.view(d).kind() {
            DefKind::Enum => {
                let idx = self.env.view(d).enum_position(name);
                match idx {
                    Some(i) => Some(self.env.types.intern(Ty::EnumMember(d, i as u32))),
                    None => {
                        if !quiet {
                            let msg = format!(
                                "enum `{}` has no member `{name}`",
                                self.env.view(d).name()
                            );
                            self.report("E2003", span, msg);
                        }
                        Some(Types::ERROR)
                    }
                }
            }
            DefKind::Namespace => match self.env.view(d).member(name).map(|m| m.ty) {
                Some(t) => Some(t),
                None => {
                    if !quiet {
                        let msg = format!("`{}` has no member `{name}`", self.env.view(d).name());
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
                    let args: Vec<TypeId> = self
                        .env
                        .view(d)
                        .params()
                        .iter()
                        .map(|_| Types::ANY)
                        .collect();
                    match self.env.instance_member(d, &args, name) {
                        Some((m, owner)) if m.kind == MemberKind::Method => {
                            if m.renamable {
                                self.private_methods.insert(span, owner);
                            }
                            let inst = self.env.instance(d);
                            Some(self.env.with_self(m.ty, inst))
                        }
                        _ => {
                            if !quiet && !self.env.view(d).native() {
                                let msg = format!(
                                    "class `{}` has no static member `{name}`",
                                    self.env.view(d).name()
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
                Ty::Named(d, args) if self.env.view(d).kind() != DefKind::Enum => {
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
            Ty::Func(f) => {
                let mut f = std::sync::Arc::unwrap_or_clone(f);
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
        let string_ns = self.env.declared.global("string")?;
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

    fn index_type(&mut self, o: TypeId, k: TypeId, key: Expr<'_>, span: Span) -> TypeId {
        if let ExprKind::String(s) = key.kind() {
            let name = String::from_utf8_lossy(&s.value()).into_owned();
            return self
                .member_type(o, &name, key.span(), false)
                .unwrap_or(Types::ANY);
        }
        self.check_secret(k, key.span());
        let mut out = Vec::new();
        for m in self.env.types.members(o) {
            let t = match self.env.types.get(m).clone() {
                Ty::Array(e) => {
                    self.check_assignable(key.span(), k, Types::NUMBER);
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
                _ if self.unindexable(m) => {
                    let msg = format!("cannot index a value of type `{}`", self.env.display(m));
                    self.report("E2003", key.span(), msg);
                    Types::ERROR
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
        lhs: Expr<'_>,
        rhs: Expr<'_>,
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
                self.and_falsy_or(lhs);
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
                // `X ?? default` tests whether `X` exists, like `X or default`.
                self.mark_probes(lhs);
                let l = self.check(lhs, expected);
                let r = self.check(rhs, expected);
                let non_nil = self.env.types.non_nil(l);
                self.env.types.union(&[non_nil, r])
            }
            BinOp::Eq | BinOp::Ne => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                self.check_secret(l, lhs.span());
                self.check_secret(r, rhs.span());
                Types::BOOLEAN
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                self.check_secret(l, lhs.span());
                self.check_secret(r, rhs.span());
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
                        self.report("E2001", e.span(), msg);
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
                self.check_secret(l, lhs.span());
                self.check_secret(r, rhs.span());
                self.arith_operand(l, lhs.span());
                self.arith_operand(r, rhs.span());
                if matches!(op, BinOp::Div | BinOp::IDiv | BinOp::Mod) {
                    self.zero_divisor(r, rhs.span());
                }
                Types::NUMBER
            }
        }
    }

    /// `a and nil or b` (or `false` in the middle): the `or` always takes
    /// over, so the result is always `b` (W2061, any mode).
    fn and_falsy_or(&mut self, lhs: Expr<'_>) {
        let mut l = lhs;
        while let ExprKind::Paren(inner) = l.kind() {
            l = inner;
        }
        let ExprKind::Binary {
            op: BinOp::And,
            rhs,
            ..
        } = l.kind()
        else {
            return;
        };
        let mut r = rhs;
        while let ExprKind::Paren(inner) = r.kind() {
            r = inner;
        }
        let word = match r.kind() {
            ExprKind::Nil => "nil",
            ExprKind::False => "false",
            _ => return,
        };
        self.report(
            "W2061",
            l.span(),
            format!("`a and {word} or b` is always `b`: the `{word}` makes the `or` take over (write `not a and b or {word}`, or an `if`)"),
        );
    }

    /// A divisor that is the literal `0` (or a `const` of it): WoW raises
    /// "Division by zero" where Lua elsewhere gives inf or nan (W2060,
    /// decision S38).
    pub fn zero_divisor(&mut self, divisor: TypeId, span: Span) {
        if let Ty::Num(bits) = self.env.types.get(divisor)
            && f64::from_bits(*bits) == 0.0
        {
            self.report(
                "W2060",
                span,
                "division by zero: WoW raises an error here, where Lua elsewhere gives inf or nan (use `math.huge` for infinity)",
            );
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
                    | Ty::ClassOfParam(_)
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

    fn args_types(&mut self, args: Args<'_>, expected: &[Option<TypeId>]) -> Pack {
        match args.kind() {
            ArgsKind::Parens { args, .. } => self.expr_pack(args, expected),
            ArgsKind::Table(e) | ArgsKind::String(e) => {
                Pack::single(self.check(e, expected.first().copied().flatten()))
            }
        }
    }

    fn arg_exprs(args: Args<'_>) -> Exprs<'_> {
        args.exprs()
    }

    /// A call of `callee` (with `receiver` for `:` calls on non-method
    /// members already handled). Returns the results.
    /// `coroutine.yield` inside a function passed to `pcall`, `xpcall` or
    /// a C callback cannot yield across that boundary (W2057).
    fn check_yield_boundary(&mut self, callee: Expr<'_>, args: Args<'_>) {
        let name = match callee.kind() {
            ExprKind::Name(n) => n.text().to_string(),
            ExprKind::Field { obj, name } => match obj.kind() {
                ExprKind::Name(o) => format!("{}.{}", o.text(), name.text()),
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
            let ExprKind::Function(f) = a.kind() else {
                continue;
            };
            let mut found = None;
            for s in f.body().stmts() {
                wowlua_syntax::visit::walk_stmt(s, &mut |e| {
                    if found.is_none()
                        && let ExprKind::Call { callee, .. } = e.kind()
                        && let ExprKind::Field { obj, name } = callee.kind()
                        && name.text() == "yield"
                        && matches!(obj.kind(), ExprKind::Name(c) if c.text() == "coroutine")
                    {
                        found = Some(e.span());
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
        args: Args<'_>,
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
    fn call_quiet(&mut self, callee: TypeId, _args: Args<'_>) -> Pack {
        match self.env.types.get(callee) {
            Ty::Func(f) => f.ret.clone(),
            _ => Pack::any(),
        }
    }

    fn call_one(
        &mut self,
        callee: TypeId,
        args: Args<'_>,
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
            | Ty::Param(_)
            | Ty::ClassOfParam(_)
            | Ty::EventArgs(_) => {
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
        args: Args<'_>,
        span: Span,
        expected: Option<TypeId>,
    ) -> Pack {
        let exprs = Self::arg_exprs(args);
        // Argument types without contextual typing (function expressions
        // match any function type).
        let saved_diags = self.diagnostics.len();
        let mut arg_types: Vec<TypeId> = exprs
            .iter()
            .map(|e| {
                if matches!(e.kind(), ExprKind::Function(_)) {
                    Types::FUNCTION
                } else {
                    self.peek_or_check(e)
                }
            })
            .collect();
        // A last argument that gives several values (`unpack(t)`, `f()`,
        // `...`) fills the parameters after it too, as in Lua.
        let mut tail: Option<TypeId> = None;
        if let Some(last) = exprs.last()
            && is_multi(last)
        {
            let saved = self.flow.clone();
            let pack = self.infer_multi(last);
            self.flow = saved;
            arg_types.pop();
            arg_types.extend(pack.types.iter().copied());
            tail = match pack.rest {
                Some(Rest::Variadic(t)) => Some(t),
                Some(Rest::Pack(_)) => Some(Types::ANY),
                None => None,
            };
        }
        self.diagnostics.truncate(saved_diags);
        for &o in overloads {
            let Ty::Func(f) = self.env.types.get(o).clone() else {
                continue;
            };
            if self.accepts(&f, &arg_types, tail) {
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

    fn peek_or_check(&mut self, e: Expr<'_>) -> TypeId {
        match self.expr_types.get(&(e.span().start, e.span().end)) {
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

    /// Whether `f` accepts arguments of these types (overload filtering);
    /// `tail`: any number of values of that type follow them.
    fn accepts(&mut self, f: &Func, args: &[TypeId], tail: Option<TypeId>) -> bool {
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
        match tail {
            // The values a multiple value gives may fill the rest.
            Some(t) => {
                let t = self.env.types.non_nil(t);
                let rest: Vec<TypeId> = f.params.iter().skip(args.len()).map(|p| p.ty).collect();
                rest.into_iter()
                    .all(|p| self.env.assignable(t, p, self.mode))
            }
            None => f.params.iter().skip(args.len()).all(|p| p.optional),
        }
    }

    /// `f` with its type parameters as `any`.
    /// `...EventArgs<"NAME">` with a declared event or message `NAME`:
    /// its payload's parameters (decision L44), here and in the parameters
    /// that are functions (a handler's arguments).
    fn expand_event_args(&mut self, mut f: Func, depth: u32) -> Func {
        if depth > 4 {
            return f;
        }
        if let Some(Rest::Variadic(r)) = f.rest
            && let Ty::EventArgs(e) = self.env.types.get(r)
            && let Ty::Str(name) = self.env.types.get(*e)
            && let Some(h) = self.env.declared.handler(name).cloned()
        {
            f.params.extend(h.params);
            f.rest = h.rest;
        }
        for i in 0..f.params.len() {
            let t = f.params[i].ty;
            if let Some(n) = self.expand_event_args_in(t, depth) {
                f.params[i].ty = n;
            }
        }
        f
    }

    /// `t` with `expand_event_args` applied to its functions, if it has any
    /// to expand.
    fn expand_event_args_in(&mut self, t: TypeId, depth: u32) -> Option<TypeId> {
        match self.env.types.get(t).clone() {
            Ty::Func(g) if Self::mentions_event_args(&self.env.types, &g) => {
                let g = self.expand_event_args(std::sync::Arc::unwrap_or_clone(g), depth + 1);
                Some(self.env.types.func(g))
            }
            Ty::Union(m) => {
                let mut changed = false;
                let members: Vec<TypeId> = m
                    .iter()
                    .map(|&x| match self.expand_event_args_in(x, depth) {
                        Some(n) => {
                            changed = true;
                            n
                        }
                        None => x,
                    })
                    .collect();
                changed.then(|| self.env.types.union(&members))
            }
            _ => None,
        }
    }

    fn mentions_event_args(types: &Types, f: &Func) -> bool {
        let is = |t: TypeId| match types.get(t) {
            Ty::EventArgs(_) => true,
            Ty::Func(g) => Self::mentions_event_args(types, g),
            Ty::Union(m) => m.iter().any(
                |&x| matches!(types.get(x), Ty::Func(g) if Self::mentions_event_args(types, g)),
            ),
            _ => false,
        };
        matches!(f.rest, Some(Rest::Variadic(r)) if is(r)) || f.params.iter().any(|p| is(p.ty))
    }

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

    /// `expect(x).name` for a custom matcher of `wowlua:test` (decision
    /// T220): a function `name` in scope taking the value first (made with
    /// `matcher("name", fn)`); the value is checked against its first
    /// parameter (E8121) and the call takes the rest.
    fn custom_matcher(&mut self, obj: Expr<'_>, o: TypeId, name: Name<'_>) -> Option<TypeId> {
        let mut target = obj;
        if let ExprKind::Field {
            obj: inner,
            name: n,
        } = target.kind()
            && n.text() == "never"
        {
            target = inner;
        }
        let ExprKind::Call {
            callee: expect,
            args: expect_args,
        } = target.kind()
        else {
            return None;
        };
        if self.intrinsic_of(expect).as_deref() != Some("test.expect")
            || BUILTIN_MATCHERS.contains(&name.text())
        {
            return None;
        }
        let _ = o;
        let v = self.lookup_var(name.text())?;
        let Ty::Func(f) = self.env.types.get(self.var_type(v)).clone() else {
            return None;
        };
        let first = f.params.first()?;
        if let Some(subject) = expect_args.exprs().first()
            && let Some(&t) = self
                .expr_types
                .get(&(subject.span().start, subject.span().end))
            && !self.env.assignable(t, first.ty, self.mode)
        {
            let msg = format!(
                "`{}` takes `{}`; the value is `{}`",
                name.text(),
                self.env.display(first.ty),
                self.env.display(t)
            );
            self.report("E8121", name.span(), msg);
        }
        let call = Func {
            type_params: Vec::new(),
            params: f.params[1..].to_vec(),
            rest: f.rest.clone(),
            ret: Pack::empty(),
            predicate: None,
            unannotated: false,
        };
        Some(self.env.types.func(call))
    }

    /// `expect(x).matcher(…)` of `wowlua:test` (decision T214): matchers
    /// that need a kind of value check `x`'s type (E8121), and
    /// `toHaveBeenCalledWith` takes the spied function's parameters.
    fn matcher_call_type(&mut self, callee: Expr<'_>, c: TypeId) -> Option<TypeId> {
        let ExprKind::Field { obj, name } = callee.kind() else {
            return None;
        };
        let mut target = obj;
        if let ExprKind::Field {
            obj: inner,
            name: n,
        } = target.kind()
            && n.text() == "never"
        {
            target = inner;
        }
        let ExprKind::Call {
            callee: expect,
            args: expect_args,
        } = target.kind()
        else {
            return None;
        };
        if self.intrinsic_of(expect).as_deref() != Some("test.expect") {
            return None;
        }
        let subject = expect_args.exprs().first()?;
        let t = *self
            .expr_types
            .get(&(subject.span().start, subject.span().end))?;
        let matcher = name.text();
        let (fits, what): (fn(&Ty) -> bool, &str) = match matcher {
            "toHaveLength" | "toContain" => (
                |ty| {
                    matches!(
                        ty,
                        Ty::String
                            | Ty::Str(_)
                            | Ty::Table
                            | Ty::Array(_)
                            | Ty::Map(..)
                            | Ty::Record(_)
                            | Ty::Named(..)
                    )
                },
                "a string or a table",
            ),
            "toBeCloseTo" => (|ty| matches!(ty, Ty::Number | Ty::Num(_)), "a number"),
            "toThrow"
            | "toBeBlocked"
            | "toHaveBeenCalled"
            | "toHaveBeenCalledTimes"
            | "toHaveBeenCalledWith" => (
                |ty| matches!(ty, Ty::Function | Ty::Func(_) | Ty::Intersection(_)),
                "a function",
            ),
            // A frame typed by its mixins (`MerchantFrame`) is a widget.
            "toBeShown" | "toBeVisible" | "toHaveText" | "toHaveSize" | "toBeAnchoredTo"
            | "toHaveLines" => (
                |ty| {
                    matches!(
                        ty,
                        Ty::Table
                            | Ty::Record(_)
                            | Ty::Named(..)
                            | Ty::Map(..)
                            | Ty::Intersection(_)
                    )
                },
                "a widget",
            ),
            _ => return None,
        };
        let members: Vec<TypeId> = match self.env.types.get(t) {
            Ty::Union(m) => m.clone(),
            _ => vec![t],
        };
        let ok = members.iter().all(|&m| {
            let ty = self.env.types.get(m);
            matches!(
                ty,
                Ty::Any | Ty::Unknown | Ty::Error | Ty::Param(_) | Ty::Nil
            ) || fits(ty)
        });
        if !ok {
            let msg = format!(
                "`{matcher}` needs {what}; the value is `{}`",
                self.env.display(t)
            );
            self.report("E8121", callee.span(), msg);
            return Some(c);
        }
        let spied_type = self.env.types.non_nil(t);
        if matcher == "toHaveBeenCalledWith"
            && let Ty::Func(spied) = self.env.types.get(spied_type).clone()
        {
            let f = Func {
                type_params: Vec::new(),
                params: spied.params.clone(),
                rest: spied.rest.clone(),
                ret: Pack::empty(),
                predicate: None,
                unannotated: false,
            };
            return Some(self.env.types.func(f));
        }
        None
    }

    /// `saved.bind(name, { defaults = … })` returns the data table: the
    /// type of the defaults (spec stdlib "saved").
    fn saved_bind_type(&mut self, callee: Expr<'_>, args: Args<'_>) -> Option<TypeId> {
        if self.intrinsic_of(callee).as_deref() != Some("saved.bind") {
            return None;
        }
        let ExprKind::Table(fields) = &args.exprs().get(1)?.kind() else {
            return None;
        };
        fields.iter().find_map(|f| match f {
            TableField::Named { name, value, .. } if name.text() == "defaults" => self
                .expr_types
                .get(&(value.span().start, value.span().end))
                .copied(),
            _ => None,
        })
    }

    /// `L.KEY(params)` of `wowlua:locale` with a value that may be secret:
    /// the string made from it is secret too (spec stdlib "locale").
    fn secret_locale_call(&mut self, callee: Expr<'_>, args: Args<'_>) -> bool {
        let ExprKind::Field { obj, .. } = callee.kind() else {
            return false;
        };
        if self.intrinsic_of(obj).as_deref() != Some("locale.L") {
            return false;
        }
        args.exprs().first().is_some_and(|a| {
            self.expr_types
                .get(&(a.span().start, a.span().end))
                .is_some_and(|&t| {
                    self.secret_path(t, String::new(), &mut std::collections::HashSet::new())
                        .is_some()
                })
        })
    }

    /// Checks of standard-library calls that need their arguments' types:
    /// `Secret<…>` in `saved.bind` defaults (E8104), hook targets (W8040).
    fn stdlib_after_call(&mut self, callee: Expr<'_>, args: Args<'_>) {
        let Some(id) = self.intrinsic_of(callee) else {
            return;
        };
        if id.starts_with("hooks.") {
            self.check_hook_target(&id, args.exprs(), false);
        }
        if id == "mock.module" {
            self.check_mock_module(args.exprs());
        }
        if id == "settings.register"
            && let Some(def) = args.exprs().first()
        {
            self.check_settings(def);
        }
        if id == "frames.movable"
            && let Some(opts) = args.exprs().get(1)
        {
            self.check_movable(opts);
        }
        if id == "saved.bind"
            && let Some(opts) = args.exprs().get(1)
            && let ExprKind::Table(fields) = opts.kind()
        {
            for f in fields {
                if let TableField::Named { name, value, .. } = f
                    && name.text() == "defaults"
                    && let Some(&t) = self.expr_types.get(&(value.span().start, value.span().end))
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
                    self.report("E8104", value.span(), msg);
                }
            }
        }
    }

    /// `mock.module(namespace, { name = value, … })` (decision T224): each
    /// replacement is an export of the module (E8123) of its type.
    fn check_mock_module(&mut self, args: Exprs<'_>) {
        let (Some(ns), Some(repl)) = (args.first(), args.get(1)) else {
            return;
        };
        let Some(&ns_type) = self.expr_types.get(&(ns.span().start, ns.span().end)) else {
            return;
        };
        let Ty::Record(record) = self.env.types.get(ns_type).clone() else {
            return;
        };
        let ExprKind::Table(fields) = repl.kind() else {
            return;
        };
        for f in fields {
            let TableField::Named { name, value, .. } = f else {
                continue;
            };
            match record.fields.get(name.text()) {
                None => {
                    let msg = format!("the module has no export `{}`", name.text());
                    self.report("E8123", name.span(), msg);
                }
                Some(field) => {
                    if let Some(&t) = self.expr_types.get(&(value.span().start, value.span().end)) {
                        self.check_assignable(value.span(), t, field.ty);
                    }
                }
            }
        }
    }

    /// `frames.movable(frame, { store = …, key = "…" })`: the store's field
    /// holds an optional position (E8340): `nil`, or a table with
    /// `point`, `relativePoint`, `x` and `y`.
    fn check_movable(&mut self, opts: Expr<'_>) {
        let ExprKind::Table(fields) = opts.kind() else {
            return;
        };
        let field = |name: &str| {
            fields.iter().find_map(|f| match f {
                TableField::Named { name: n, value, .. } if n.text() == name => Some(value),
                _ => None,
            })
        };
        let (Some(store), Some(key_expr)) = (field("store"), field("key")) else {
            return;
        };
        let ExprKind::String(lit) = key_expr.kind() else {
            return;
        };
        let Some(&store_type) = self.expr_types.get(&(store.span().start, store.span().end)) else {
            return;
        };
        let key = String::from_utf8_lossy(&lit.value()).into_owned();
        let found = match self.env.types.get(store_type).clone() {
            Ty::Named(d, targs) => self.env.instance_member(d, &targs, &key).map(|(m, _)| m.ty),
            Ty::Record(r) => r.fields.get(key.as_str()).map(|f| f.ty),
            _ => return,
        };
        let fits = found.is_some_and(|t| {
            let value = self.env.types.non_nil(t);
            matches!(self.env.types.get(value), Ty::Any | Ty::Unknown | Ty::Error)
                || (self.env.types.is_optional(t)
                    && ["point", "relativePoint", "x", "y"].iter().all(|m| {
                        matches!(self.member_type(value, m, key_expr.span(), true), Some(t) if t != Types::ERROR)
                    }))
        });
        if !fits {
            let shown = found.map_or("missing".to_string(), |t| {
                format!("`{}`", self.env.display(t))
            });
            let msg = format!(
                "the store's `{key}` is {shown}; a movable frame keeps its position in a `FramePosition?` field"
            );
            self.report("E8340", key_expr.span(), msg);
        }
    }

    /// `settings.register({ store = …, controls = { … } })`: every control's
    /// key is a field of the store (E8113) of the control's type (E8070),
    /// and dropdown values are of the field's type (E8107).
    fn check_settings(&mut self, def: Expr<'_>) {
        let ExprKind::Table(fields) = def.kind() else {
            return;
        };
        let field = |name: &str| {
            fields.iter().find_map(|f| match f {
                TableField::Named { name: n, value, .. } if n.text() == name => Some(value),
                _ => None,
            })
        };
        let Some(store) = field("store") else { return };
        let Some(&store_type) = self.expr_types.get(&(store.span().start, store.span().end)) else {
            return;
        };
        if matches!(
            self.env.types.get(store_type),
            Ty::Any | Ty::Unknown | Ty::Error | Ty::Table
        ) {
            return;
        }
        let mut lists: Vec<Expr<'_>> = field("controls").into_iter().collect();
        if let Some(subs) = field("subcategories")
            && let ExprKind::Table(entries) = subs.kind()
        {
            for e in entries {
                if let TableField::Positional(sub) = e
                    && let ExprKind::Table(sub_fields) = sub.kind()
                {
                    lists.extend(sub_fields.iter().filter_map(|f| match f {
                        TableField::Named { name, value, .. } if name.text() == "controls" => {
                            Some(value)
                        }
                        _ => None,
                    }));
                }
            }
        }
        for list in lists {
            let ExprKind::Table(entries) = list.kind() else {
                continue;
            };
            for e in entries {
                let TableField::Positional(control) = e else {
                    continue;
                };
                if let ExprKind::Call { callee, args } = control.kind() {
                    self.check_settings_control(store_type, callee, args.exprs());
                }
            }
        }
    }

    fn check_settings_control(&mut self, store: TypeId, callee: Expr<'_>, args: Exprs<'_>) {
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
        let Some(key_expr) = args.first() else {
            return;
        };
        let ExprKind::String(lit) = key_expr.kind() else {
            return;
        };
        let key = String::from_utf8_lossy(&lit.value()).into_owned();
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
            self.report("E8113", key_expr.span(), msg);
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
                matches!(self.member_type(value, c, key_expr.span(), true), Some(t)
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
            self.report("E8070", key_expr.span(), msg);
            return;
        }
        if kind == "dropdown"
            && let Some(opts) = args.get(1)
            && let ExprKind::Table(opt_fields) = opts.kind()
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
                if name.text() != "options" {
                    continue;
                }
                let ExprKind::Table(entries) = options.kind() else {
                    continue;
                };
                for entry in entries {
                    let TableField::Positional(option) = entry else {
                        continue;
                    };
                    let ExprKind::Table(parts) = option.kind() else {
                        continue;
                    };
                    for p in parts {
                        if let TableField::Named { name, value: v, .. } = p
                            && name.text() == "value"
                            && let Some(&t) = self.expr_types.get(&(v.span().start, v.span().end))
                            && !self.env.assignable(t, field, self.mode)
                        {
                            let msg = format!(
                                "the option value `{}` is not a `{}` (the type of `{key}`)",
                                self.env.display(t),
                                self.env.display(field)
                            );
                            self.report("E8107", v.span(), msg);
                        }
                    }
                }
            }
        }
    }

    /// W8102: the target belongs to a load-on-demand addon, but the hook
    /// does not wait for it (`opts.addon`), or names it by an expression
    /// (evaluated before the addon loads).
    fn check_hook_on_demand(&mut self, id: &str, args: Exprs<'_>, decorator: bool) {
        let Some(first) = args.first() else { return };
        let (root, by_name) = match first.kind() {
            ExprKind::String(lit) => (String::from_utf8_lossy(&lit.value()).into_owned(), true),
            ExprKind::Name(n) if self.lookup_var(n.text()).is_none() => {
                (n.text().to_string(), false)
            }
            _ => return,
        };
        let Some(addon) = self
            .env
            .declared
            .on_demand(root.as_str())
            .map(str::to_string)
        else {
            return;
        };
        let has_method = matches!(args.get(1).map(|e| e.kind()), Some(ExprKind::String(_)));
        let global_only = id == "hooks.hook" && !has_method;
        let opts_at = if decorator { 2 } else { 3 };
        let waits = !global_only
            && match args.get(opts_at).map(|e| e.kind()) {
                Some(ExprKind::Table(fields)) => fields
                    .iter()
                    .any(|f| matches!(f, TableField::Named { name, .. } if name.text() == "addon")),
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
        self.report("W8102", first.span(), msg);
    }

    /// Whether an enclosing existence test checked the hooked member:
    /// `if _G.Name then` (or `if Name then`) for `hook("Name", fn)`, `if
    /// obj.Method then` for `hook(obj, "Method", fn)`.
    fn hook_target_tested(&self, target: Expr<'_>, method: Option<&str>) -> bool {
        // `_G.X` and `X` name the same global.
        let global = |path: &str| path.strip_prefix("_G.").unwrap_or(path).to_string();
        let path = match target.kind() {
            ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value()).into_owned()),
            _ => self.api_path_of(target),
        };
        if let Some(path) = path {
            let key = match method {
                Some(m) => global(&format!("{path}.{m}")),
                None => global(&path),
            };
            return self.guard.present.iter().any(|p| global(p) == key);
        }
        let Some(method) = method else { return false };
        match target.kind() {
            ExprKind::Name(n) => self
                .lookup_var(n.text())
                .is_some_and(|v| self.guard.fields.contains(&(v, method.to_string()))),
            _ => self.field_path(target).is_some_and(|(root, path)| {
                self.guard
                    .fields
                    .contains(&(root, format!("{path}.{method}")))
            }),
        }
    }

    /// A hook's target exists (W8040): `hook(name, fn)` and `@hook(name)`
    /// name a global function, `hook(target, method, …)` a method of the
    /// target, `hookScript(frame, …)` a frame. An enclosing existence test
    /// of the member stands for its declaration. A target of a
    /// load-on-demand addon needs `opts.addon` and a name (W8102), tested or
    /// not.
    pub(crate) fn check_hook_target(&mut self, id: &str, args: Exprs<'_>, decorator: bool) {
        let literal = |e: Expr<'_>| match e.kind() {
            ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value()).into_owned()),
            _ => None,
        };
        self.check_hook_on_demand(id, args, decorator);
        let known = |c: &Self, name: &str| {
            c.env.declared.has_global(name)
                || c.extra_globals.contains(name)
                || c.assigned_globals.contains(name)
                || c.dynamic_env
        };
        let Some(first) = args.first() else { return };
        let method = args.get(1).and_then(literal);
        // `hookScript`'s target is the frame itself.
        let member = (id != "hooks.hookScript")
            .then_some(method.as_deref())
            .flatten();
        if self.hook_target_tested(first, member) {
            return;
        }
        let global_only = id == "hooks.hook" && method.is_none();
        if global_only || id == "hooks.hookScript" {
            if let Some(name) = literal(first)
                && !known(self, &name)
            {
                let what = if global_only { "function" } else { "frame" };
                let msg = format!(
                    "`{name}` is not a declared global {what}; the hook fails where it is installed"
                );
                self.report("W8040", first.span(), msg);
            }
            return;
        }
        let (Some(method), Some(method_expr)) = (method, args.get(1)) else {
            return;
        };
        let target = match literal(first) {
            Some(name) => match self.env.declared.global(name.as_str()) {
                Some(t) => t,
                None => {
                    if !known(self, &name) {
                        let msg = format!(
                            "`{name}` is not a declared global; the hook fails where it is installed"
                        );
                        self.report("W8040", first.span(), msg);
                    }
                    return;
                }
            },
            None => match self.expr_types.get(&(first.span().start, first.span().end)) {
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
            // A frame typed by its mixins (`ChatFrame1`): no part declares
            // the member.
            Ty::Intersection(parts) => parts.iter().all(|&p| match self.env.types.get(p).clone() {
                Ty::Named(d, targs) => {
                    self.env.def(d).kind != DefKind::Enum
                        && self.env.instance_member(d, &targs, &method).is_none()
                        && self.env.def(d).indexer.is_none()
                }
                _ => false,
            }),
            _ => matches!(
                self.member_type(target, &method, method_expr.span(), true),
                None | Some(Types::ERROR)
            ),
        };
        if missing {
            let msg = format!(
                "`{}` has no `{method}` to hook; the hook fails where it is installed",
                self.env.display(target)
            );
            self.report("W8040", method_expr.span(), msg);
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
    fn frame_pool_type(&mut self, mut f: Func, exprs: Exprs<'_>) -> Option<TypeId> {
        let first = exprs.first()?;
        if !matches!(first.kind(), ExprKind::Name(_) | ExprKind::Field { .. }) {
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
        f.params.extend(ctor.params.iter().cloned());
        f.rest = ctor.rest.clone();
        f.ret = Pack::single(self.env.types.intern(Ty::Named(pool_def, vec![instance])));
        Some(self.env.types.func(f))
    }

    /// A standard-library call typed by its event or message: the handler
    /// of `on`, `once` and `onUnit` gets the event's parameters, and `send`
    /// takes the message's.
    /// `registry:on/once/off/offAll/fire(…)` on a `callbacks.Registry<E>`:
    /// an event name must be a literal key of `E` (E8420); the handler, or
    /// the arguments, are checked against `E[name]` (spec stdlib
    /// "callbacks"). `off` takes a subscription, or a name and a handler.
    fn registry_method_type(
        &mut self,
        o: TypeId,
        method: &str,
        args: Args<'_>,
        m: TypeId,
    ) -> TypeId {
        if !matches!(method, "on" | "once" | "off" | "offAll" | "fire") {
            return m;
        }
        let Ty::Named(d, targs) = self.env.types.get(o).clone() else {
            return m;
        };
        if self.env.declared.callbacks_registry() != Some(d) {
            return m;
        }
        let Some(&events) = targs.first() else {
            return m;
        };
        if events == Types::ANY || matches!(self.env.types.get(events), Ty::Param(_)) {
            return m;
        }
        let exprs = args.exprs();
        // `offAll(owner, name?)`: the name is the second argument.
        let at = if method == "offAll" { 1 } else { 0 };
        let Some(first) = exprs.get(at) else {
            return m;
        };
        let ExprKind::String(lit) = first.kind() else {
            if method == "off" {
                // `off(sub)`: a subscription, and nothing after it.
                return self.registry_off_subscription(m);
            }
            let msg = format!(
                "the event name of `{method}` must be a string literal naming a key of `{}`",
                self.env.display(events)
            );
            self.report("E8420", first.span(), msg);
            return m;
        };
        let event = String::from_utf8_lossy(&lit.value()).into_owned();
        let Some(handler) = self.field_type_quiet(events, &event) else {
            let msg = format!(
                "`{event}` is not an event of `{}`",
                self.env.display(events)
            );
            self.report("E8420", first.span(), msg);
            return m;
        };
        if method == "offAll" {
            return m;
        }
        let (Ty::Func(f), Some(h)) = (self.env.types.get(m).clone(), self.as_func(handler)) else {
            return m;
        };
        let mut f = std::sync::Arc::unwrap_or_clone(f);
        if method == "fire" {
            f.params.truncate(1);
            f.params.extend(h.params);
            f.rest = h.rest;
        } else if let Some(p) = f.params.get_mut(1) {
            p.ty = handler;
            if method == "off" {
                // `off(name, handler)`: a name needs its handler.
                p.optional = false;
                f.params[0].ty = Types::STRING;
            }
        }
        self.env.types.func(f)
    }

    /// `router:add(name, command, owner?)` on a `slash.Router`: a literal
    /// command is checked as `router`'s are (E8112, spec stdlib "slash").
    fn check_router_add(&mut self, o: TypeId, method: &str, args: Args<'_>) {
        if method != "add" {
            return;
        }
        let Ty::Named(d, _) = self.env.types.get(o).clone() else {
            return;
        };
        if self.env.declared.slash_router() != Some(d) {
            return;
        }
        let Some(command) = args.exprs().get(1) else {
            return;
        };
        for (span, message, _) in crate::stdlib::router_command_problems(command) {
            self.report("E8112", span, message);
        }
    }

    /// `registry:off(sub)`: `off`'s type without the name form, so that a
    /// name that is no literal, or a handler after a subscription, is
    /// reported.
    fn registry_off_subscription(&mut self, m: TypeId) -> TypeId {
        let Ty::Func(f) = self.env.types.get(m).clone() else {
            return m;
        };
        let mut f = std::sync::Arc::unwrap_or_clone(f);
        let Some(target) = f.params.first().map(|p| p.ty) else {
            return m;
        };
        if let Ty::Union(ms) = self.env.types.get(target).clone() {
            let rest: Vec<TypeId> = ms.iter().copied().filter(|&t| t != Types::STRING).collect();
            if let [sub] = rest.as_slice() {
                f.params[0].ty = *sub;
            }
        }
        f.params.truncate(1);
        self.env.types.func(f)
    }

    /// `watchUnit(names, handler, owner?)`: with a literal list of names,
    /// the handler gets one of them first (as with several `@event` names),
    /// then anything.
    fn watch_unit_type(&mut self, mut f: Func, exprs: Exprs<'_>) -> Option<TypeId> {
        let ExprKind::Table(fields) = exprs.first()?.kind() else {
            return None;
        };
        let mut names = Vec::new();
        for field in fields {
            let TableField::Positional(v) = field else {
                return None;
            };
            let ExprKind::String(lit) = v.kind() else {
                return None;
            };
            let name = String::from_utf8_lossy(&lit.value()).into_owned();
            self.check_event_available(&name, v.span());
            names.push(self.env.types.str(&name));
        }
        if names.is_empty() {
            return None;
        }
        let event = self.env.types.union(&names);
        let handler = self.env.types.func(Func {
            type_params: Vec::new(),
            params: vec![super::types::Param {
                name: Some("event".into()),
                ty: event,
                optional: false,
            }],
            rest: Some(Rest::Variadic(Types::ANY)),
            ret: Pack::any(),
            predicate: None,
            unannotated: false,
        });
        f.params.get_mut(1)?.ty = handler;
        Some(self.env.types.func(f))
    }

    fn stdlib_call_type(&mut self, callee: Expr<'_>, args: Args<'_>, c: TypeId) -> TypeId {
        if let Some(t) = self.matcher_call_type(callee, c) {
            return t;
        }
        let Some(id) = self.intrinsic_of(callee) else {
            return c;
        };
        let Ty::Func(f) = self.env.types.get(c).clone() else {
            return c;
        };
        let mut f = std::sync::Arc::unwrap_or_clone(f);
        let exprs = args.exprs();
        if id == "pool.FramePool.new" {
            return self.frame_pool_type(f, exprs).unwrap_or(c);
        }
        if id == "events.onBucket" {
            let names: Vec<Expr<'_>> = match exprs.first().map(|e| e.kind()) {
                Some(ExprKind::Table(fields)) => fields
                    .iter()
                    .filter_map(|f| match f {
                        TableField::Positional(v) => Some(v),
                        _ => None,
                    })
                    .collect(),
                Some(_) => vec![exprs.at(0)],
                None => Vec::new(),
            };
            for e in names {
                if let ExprKind::String(lit) = e.kind() {
                    self.check_event_available(&String::from_utf8_lossy(&lit.value()), e.span());
                }
            }
            return c;
        }
        if id == "events.watchUnit" {
            return self.watch_unit_type(f, exprs).unwrap_or(c);
        }
        let (name, name_span) = match exprs.first().map(|e| (e.kind(), e.span())) {
            Some((ExprKind::String(lit), span)) => {
                (String::from_utf8_lossy(&lit.value()).into_owned(), span)
            }
            _ => return c,
        };
        if matches!(id.as_str(), "events.on" | "events.once" | "events.onUnit") {
            self.check_event_available(&name, name_span);
        }
        let Some(handler) = self.env.declared.handler(name.as_str()).cloned() else {
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
        args: Args<'_>,
        span: Span,
        expected: Option<TypeId>,
        report: bool,
    ) -> Option<Pack> {
        let exprs = Self::arg_exprs(args);
        let generic = !f.type_params.is_empty();
        // First pass: infer type arguments from the non-function arguments.
        let mut s = Subst::default();
        // Arguments from this position on were checked as a call of a
        // function passed with them (`pcall(f, a, b)`).
        let mut passed_from: Option<usize> = None;
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
                if let ExprKind::Function(lit) = e.kind() {
                    // A literal with every parameter annotated gives its
                    // parameters' types (`spy(function(a: number) end)`);
                    // its body is checked in the second pass.
                    let annotated = lit.type_params().is_empty()
                        && lit.params().iter().all(|p| p.ty().is_some())
                        && lit.vararg().is_none_or(|v| v.ty.is_some());
                    if annotated && let Some(p) = self.param_at(f, i) {
                        let mut sig =
                            self.env
                                .func_sig(&FnSig::of(lit), &self.scope.clone(), false);
                        if lit.ret().is_none() {
                            sig.ret = Pack::any();
                        }
                        let t = self.env.types.func(sig);
                        self.unify(p, t, &mut s);
                    }
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
            // Expected results give type arguments too: a parameter the
            // arguments left open, or one whose binding fits the expected
            // one (`local xs: (number | string)[] = list(1)`, which strict
            // mode's invariance needs). A binding that does not fit stays,
            // so the error names the value's type, not a union with the
            // target (`id(5)` for a `boolean`: `number?`).
            if let (Some(exp), Some(&r)) = (expected, f.ret.types.first()) {
                let mut from_exp = Subst::default();
                self.unify(r, exp, &mut from_exp);
                for (p, t) in from_exp.types {
                    match s.types.get(&p).copied() {
                        Some(bound)
                            if !s.weak.contains(&p)
                                && !self.env.assignable(bound, t, self.mode) => {}
                        _ => {
                            s.weak.remove(&p);
                            s.types.insert(p, t);
                        }
                    }
                }
                s.packs.extend(from_exp.packs);
            }
            // The variadic tail binds a pack parameter. A function literal
            // (typed in the second pass) takes the type the pack was bound
            // to at its position by an earlier parameter, or `any`.
            // A function of unknown signature (`function`, `any`) for a
            // `(A...) -> …` parameter: `A` is anything.
            let mut open_packs: Vec<ParamId> = Vec::new();
            for (i, t) in arg_types.iter().enumerate() {
                let (Some(t), Some(pt)) = (t, self.param_at(f, i)) else {
                    continue;
                };
                if matches!(self.env.types.get(*t), Ty::Function | Ty::Any)
                    && let Ty::Func(pf) = self.env.types.get(pt).clone()
                    && let Some(Rest::Pack(p)) = &pf.rest
                {
                    open_packs.push(*p);
                }
            }
            for p in &open_packs {
                s.packs.insert(*p, Pack::any());
            }
            if let Some(Rest::Pack(p)) = &f.rest
                && !open_packs.contains(p)
            {
                let fixed = f.params.len();
                let bound = s.packs.get(p).cloned();
                let mut pack = Pack::default();
                for (k, t) in arg_types.iter().skip(fixed).enumerate() {
                    // A final `...` or call passes any number of values.
                    if fixed + k + 1 == exprs.len() && is_multi(exprs.at(fixed + k)) {
                        pack.rest = Some(Rest::Variadic(t.unwrap_or(Types::ANY)));
                        break;
                    }
                    let from_bound = || bound.as_ref().and_then(|b| b.types.get(k).copied());
                    pack.types.push(t.or_else(from_bound).unwrap_or(Types::ANY));
                }
                // A function passed on with its arguments (`pcall(f, a, b)`):
                // the arguments are checked as a call of it, so a missing
                // one is E2007 rather than a mismatch of the function.
                let passed = bound.is_some() && pack.rest.is_none() && report;
                if passed
                    && let Some(af) = self.passed_function(f, &arg_types, *p)
                    && self.check_passed_args(&af, &exprs, fixed, &arg_types, span)
                {
                    s.packs.insert(*p, bound.expect("bound"));
                    passed_from = Some(fixed);
                } else {
                    s.packs.insert(*p, pack);
                }
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
        let f = self.expand_event_args(f, 0);
        // Second pass: check every argument against its parameter.
        let n_args = exprs.len();
        for (i, e) in exprs.iter().enumerate() {
            let p = self.param_at(&f, i);
            let multi = i + 1 == n_args && is_multi(e);
            let t = match arg_types[i] {
                Some(t) if !matches!(e.kind(), ExprKind::Function(_)) => t,
                _ => {
                    if multi {
                        let pack = self.expr_inner(e, p).into_pack();
                        // `...T` alone: its first value is a `T` (or none).
                        let first = match (pack.types.first(), &pack.rest) {
                            (Some(&t), _) => t,
                            (None, Some(Rest::Variadic(t))) => *t,
                            (None, _) => Types::NIL,
                        };
                        self.expr_types
                            .insert((e.span().start, e.span().end), first);
                        // The extra values of a final multi-value argument.
                        for (j, &extra) in pack.types.iter().enumerate().skip(1) {
                            if let Some(pt) = self.param_at(&f, i + j)
                                && report
                            {
                                self.check_assignable(e.span(), extra, pt);
                            }
                        }
                        first
                    } else {
                        self.check(e, p)
                    }
                }
            };
            match p {
                _ if passed_from.is_some_and(|from| i >= from) => {}
                Some(pt) => {
                    if report {
                        self.check_assignable(e.span(), t, pt);
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
                    self.report(code, e.span(), msg);
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
        if report && supplied != usize::MAX && passed_from.is_none() {
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

    /// The function argument whose parameters bind the pack `p` (the `f` of
    /// `pcall(f, ...)`), if it has a known signature.
    fn passed_function(&self, f: &Func, arg_types: &[Option<TypeId>], p: ParamId) -> Option<Func> {
        f.params.iter().enumerate().find_map(|(j, param)| {
            let Ty::Func(pf) = self.env.types.get(param.ty) else {
                return None;
            };
            if pf.rest != Some(Rest::Pack(p)) || !pf.params.is_empty() {
                return None;
            }
            match self.env.types.get((*arg_types.get(j)?)?) {
                Ty::Func(af) if af.type_params.is_empty() => Some(Func::clone(af)),
                _ => None,
            }
        })
    }

    /// Checks the arguments from position `fixed` on as a call of `af`
    /// when they leave out one it needs (E2007, and E2001 for the given
    /// ones). Extra arguments are allowed, as before. Whether it reported.
    fn check_passed_args(
        &mut self,
        af: &Func,
        exprs: &Exprs<'_>,
        fixed: usize,
        arg_types: &[Option<TypeId>],
        span: Span,
    ) -> bool {
        let given = arg_types.len().saturating_sub(fixed);
        let Some(missing) = af.params.iter().skip(given).find(|p| !p.optional) else {
            return false;
        };
        for (k, param) in af.params.iter().enumerate().take(given) {
            let (Some(t), e) = (arg_types[fixed + k], exprs.at(fixed + k)) else {
                continue;
            };
            // Literal tables and functions take a direct call's parameter
            // type as their context; they were typed without it here.
            if !matches!(e.kind(), ExprKind::Table(_) | ExprKind::Function(_)) {
                self.check_assignable(e.span(), t, param.ty);
            }
        }
        let name = missing
            .name
            .as_deref()
            .map(|n| format!(" `{n}`"))
            .unwrap_or_default();
        let msg = format!(
            "missing argument{name} of type `{}`",
            self.env.display(missing.ty)
        );
        self.report("E2007", span, msg);
        true
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
        self.unify_at(pattern, actual, s, true, false);
    }

    /// `widen`: a literal binds as its base type (`"x"` as `string`). Not
    /// inside a table's element or value type, which is declared (literal
    /// tables widen their own entries) and invariant: `{ [string]: false }`
    /// binds `V` to `false`, not `boolean`.
    ///
    /// `contra`: `pattern` is a parameter of a function type (an odd number
    /// of them deep). A function that takes a wider type fits (parameters
    /// are contravariant), so such a binding only stands in where no other
    /// position binds the parameter: `sort(rows, function(a: { ms: number },
    /// b: { ms: number }) … end)` binds `V` to the rows' type.
    fn unify_at(
        &mut self,
        pattern: TypeId,
        actual: TypeId,
        s: &mut Subst,
        widen: bool,
        contra: bool,
    ) {
        match (
            self.env.types.get(pattern).clone(),
            self.env.types.get(actual).clone(),
        ) {
            (Ty::Param(p), _) => {
                // A constrained parameter keeps a literal (`E extends
                // EventName` binds "UNIT_HEALTH", not `string`, decision L44).
                let constrained = self.env.params[p.0 as usize].constraint.is_some();
                let widened = if widen && !constrained {
                    self.env.types.widen(actual)
                } else {
                    actual
                };
                let weak = s.weak.contains(&p);
                match s.types.get(&p).copied() {
                    // A parameter position does not widen another's binding.
                    Some(_) if contra && !weak => {}
                    // Another position replaces a parameter position's.
                    Some(_) if !contra && weak => {
                        s.weak.remove(&p);
                        s.types.insert(p, widened);
                    }
                    Some(existing) if existing != widened => {
                        let u = self.env.types.union(&[existing, widened]);
                        s.types.insert(p, u);
                    }
                    Some(_) => {}
                    None => {
                        if contra {
                            s.weak.insert(p);
                        }
                        s.types.insert(p, widened);
                    }
                }
            }
            // `ClassOf<T>` against a class's (or an enum's) table: T is its
            // instance type (the enum), any type arguments `any`.
            (Ty::ClassOfParam(p), Ty::ClassOf(d))
                if matches!(self.env.def(d).kind, DefKind::Class | DefKind::Enum) =>
            {
                let targs = vec![Types::ANY; self.env.def(d).params.len()];
                let instance = self.env.types.intern(Ty::Named(d, targs));
                let param = self.env.types.intern(Ty::Param(p));
                self.unify_at(param, instance, s, false, contra);
            }
            (Ty::Array(a), Ty::Array(b)) => self.unify_at(a, b, s, false, contra),
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.unify_at(k1, k2, s, false, contra);
                self.unify_at(v1, v2, s, false, contra);
            }
            (Ty::Map(k, v), Ty::Array(e)) => {
                self.unify_at(k, Types::NUMBER, s, true, contra);
                self.unify_at(v, e, s, false, contra);
            }
            (Ty::Map(k, v), Ty::Record(r)) => {
                if let Some((rk, rv)) = r.indexer
                    && r.fields.is_empty()
                {
                    self.unify_at(k, rk, s, false, contra);
                    self.unify_at(v, rv, s, false, contra);
                } else if let Some((rk, rv)) = r.indexer {
                    // Both an indexer and named fields (`{ [1] = true, b =
                    // true }`): the keys are the indexer's and the names.
                    let mut keys: Vec<TypeId> =
                        r.fields.keys().map(|n| self.env.types.str(n)).collect();
                    keys.push(rk);
                    let mut vals: Vec<TypeId> = r.fields.values().map(|f| f.ty).collect();
                    vals.push(rv);
                    let ku = self.env.types.union(&keys);
                    let ku = self.env.types.widen(ku);
                    let vu = self.env.types.union(&vals);
                    self.unify_at(k, ku, s, true, contra);
                    self.unify_at(v, vu, s, true, contra);
                } else if !r.fields.is_empty() {
                    let keys: Vec<TypeId> =
                        r.fields.keys().map(|n| self.env.types.str(n)).collect();
                    let vals: Vec<TypeId> = r.fields.values().map(|f| f.ty).collect();
                    let ku = self.env.types.union(&keys);
                    let ku = self.env.types.widen(ku);
                    let vu = self.env.types.union(&vals);
                    self.unify_at(k, ku, s, true, contra);
                    self.unify_at(v, vu, s, true, contra);
                }
            }
            (Ty::Union(ps), _) => {
                // `T?` against `X?`: bind T to the non-nil part.
                let non_nil: Vec<TypeId> =
                    ps.iter().copied().filter(|&x| x != Types::NIL).collect();
                if non_nil.len() == 1 {
                    let a = self.env.types.non_nil(actual);
                    self.unify_at(non_nil[0], a, s, widen, contra);
                    return;
                }
                // `T | Secret<T>` (and `nil`) against `X | Secret<X>`: the
                // secret members bind through `Secret<T>`, the rest bind T.
                let (params, others): (Vec<TypeId>, Vec<TypeId>) = non_nil
                    .iter()
                    .partition(|&&x| matches!(self.env.types.get(x), Ty::Param(_)));
                let [param] = params.as_slice() else {
                    return;
                };
                let [secret] = others.as_slice() else {
                    return;
                };
                if !matches!(self.env.types.get(*secret), Ty::Secret(_)) {
                    return;
                }
                let members = match self.env.types.get(actual).clone() {
                    Ty::Union(ms) => ms,
                    _ => vec![actual],
                };
                let mut plain = Vec::new();
                for m in members {
                    match self.env.types.get(m) {
                        Ty::Nil => {}
                        Ty::Secret(_) => self.unify_at(*secret, m, s, widen, contra),
                        _ => plain.push(m),
                    }
                }
                if !plain.is_empty() {
                    let u = self.env.types.union(&plain);
                    self.unify_at(*param, u, s, widen, contra);
                }
            }
            (Ty::Secret(p), Ty::Secret(a)) => self.unify_at(p, a, s, widen, contra),
            (Ty::Named(d1, a1), Ty::Named(d2, a2)) if d1 == d2 => {
                for (x, y) in a1.iter().zip(&a2) {
                    self.unify_at(*x, *y, s, false, contra);
                }
            }
            (Ty::Func(pf), Ty::Func(af)) => {
                for (pp, ap) in pf.params.iter().zip(&af.params) {
                    self.unify_at(pp.ty, ap.ty, s, true, !contra);
                }
                // `(A...) -> …`: the pack is the remaining parameters.
                if let Some(Rest::Pack(p)) = &pf.rest
                    && !s.packs.contains_key(p)
                {
                    let rest = Pack {
                        types: af
                            .params
                            .iter()
                            .skip(pf.params.len())
                            .map(|q| q.ty)
                            .collect(),
                        rest: af.rest.clone(),
                    };
                    s.packs.insert(*p, rest);
                }
                for (pr, ar) in pf.ret.types.iter().zip(&af.ret.types) {
                    self.unify_at(*pr, *ar, s, true, contra);
                }
                if let Some(Rest::Pack(p)) = &pf.ret.rest {
                    s.packs.insert(*p, af.ret.clone());
                }
            }
            (Ty::Record(pr), Ty::Record(ar)) => {
                for (n, f) in pr.fields.iter() {
                    if let Some(af) = ar.fields.get(n) {
                        self.unify_at(f.ty, af.ty, s, true, contra);
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
    fn api_path_of(&self, e: Expr<'_>) -> Option<String> {
        match e.kind() {
            ExprKind::Name(n) if self.lookup_var(n.text()).is_none() => Some(n.text().to_string()),
            ExprKind::Field { obj, name } => self
                .api_path_of(obj)
                .map(|p| format!("{p}.{}", name.text())),
            _ => None,
        }
    }

    /// `obj.name` is a method of a class or declared class.
    fn is_method_member(&mut self, obj: Expr<'_>, name: &str) -> bool {
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
    fn with_metatable(&mut self, t: Expr<'_>, mt: Expr<'_>) -> TypeId {
        let tt = self.peek_type(t);
        let mtt = self.peek_type(mt);
        let index = self.field_type_quiet(mtt, "__index");
        let Ty::Record(mut r) = self.env.types.get(tt).clone() else {
            return tt;
        };
        let mut fallback: FxHashSet<Box<str>> = FxHashSet::default();
        if let Some(i) = index
            && let Ty::Record(ir) = self.env.types.get(i).clone()
        {
            for (k, f) in ir.fields.iter() {
                if !r.fields.contains_key(k) {
                    r.fields.insert(k.into(), *f);
                    fallback.insert(k.into());
                }
            }
            if ir.open || r.indexer.is_none() {
                r.indexer = Some((Types::STRING, Types::ANY));
            }
        } else if index.is_some() {
            r.indexer = Some((Types::STRING, Types::ANY));
        }
        r.open = false;
        let t = self.env.types.record(r);
        if !fallback.is_empty() {
            self.fallback_fields.insert(t, fallback);
        }
        t
    }

    pub fn is_super(&self, e: Expr<'_>) -> bool {
        matches!(e.kind(), ExprKind::Name(n) if n.text() == "super")
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
pub fn is_multi(e: Expr<'_>) -> bool {
    matches!(
        e.kind(),
        ExprKind::Call { .. }
            | ExprKind::Method { .. }
            | ExprKind::OptCall { .. }
            | ExprKind::OptMethod { .. }
            | ExprKind::Vararg
    )
}

/// What a W2062 count is kept under: a variable holding the function, or
/// a file-level table's method.
#[derive(Clone, Debug)]
pub(crate) enum CapKey {
    Var(VarId),
    Method(VarId, Box<str>),
}

/// How a call's values are used: spread over names after `before` other
/// values, or `select(index, …)` after `before` other arguments.
#[derive(Clone, Debug)]
pub(crate) enum CapUse {
    Names {
        before: usize,
        names: Vec<(Span, String)>,
    },
    Select {
        before: usize,
        index: f64,
        span: Span,
    },
}

/// A method's table as written (`M`, `self`).
fn obj_text(obj: Expr<'_>) -> String {
    match obj.kind() {
        ExprKind::Name(n) => n.text().to_string(),
        _ => "?".to_string(),
    }
}
